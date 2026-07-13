//! 游戏进程管理：spawn、stdout/stderr 流式捕获、退出码回报、崩溃触发判定。
//!
//! 用 `java.exe`（而非 `javaw.exe`，规避已知的输出重定向 Bug）启动，把 Java bin 目录塞进子进程 PATH 头部
//! （某些 natives 靠系统查找同目录 DLL），并把 stdout/stderr 逐行捕获：一路投递给上层的实时消费者
//! （用于日志窗口/加载进度估算），一路进一个固定容量的环形缓冲，供进程退出后的崩溃分析取用最后若干行。

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::command::LaunchCommand;
use crate::error::{LaunchError, Result};

/// 崩溃分析缓存的最近日志行数（对齐 PCL 的 500 行）。
pub const RECENT_LINE_CAPACITY: usize = 500;

/// 日志来源流。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// 一行进程输出。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub stream: LogStream,
    pub text: String,
}

/// 进程结束报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitReport {
    /// 退出码（被信号杀死等无码情形为 `None`）。
    pub code: Option<i32>,
    /// 是否正常退出（退出码 0）。
    pub success: bool,
    /// 退出时缓存的最近若干行输出（供崩溃分析）。
    pub recent_lines: Vec<String>,
}

/// 一次已启动的游戏会话。
pub struct GameSession {
    child: Child,
    recent: Arc<Mutex<VecDeque<String>>>,
    readers: Vec<JoinHandle<()>>,
}

impl GameSession {
    /// 进程 id（尚未退出时可取）。
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// 当前缓存的最近输出行快照。
    pub fn recent_lines(&self) -> Vec<String> {
        recent_snapshot(&self.recent)
    }

    /// 强制结束进程（对应「取消启动」）。
    pub async fn kill(&mut self) -> Result<()> {
        self.child.kill().await.map_err(LaunchError::Wait)
    }

    /// 等待进程结束，回收读取任务，产出退出报告。
    pub async fn wait(mut self) -> Result<ExitReport> {
        // stdout/stderr 已被 take 走，wait 不会因未排空管道而死锁。
        let status = self.child.wait().await.map_err(LaunchError::Wait)?;
        for reader in self.readers.drain(..) {
            // 读取任务在管道关闭后自然结束；join 失败（panic）不应掩盖退出码，仅记调试日志。
            if let Err(err) = reader.await {
                tracing::debug!(error = %err, "日志读取任务异常结束");
            }
        }
        Ok(ExitReport {
            code: status.code(),
            success: status.success(),
            recent_lines: recent_snapshot(&self.recent),
        })
    }
}

/// 启动游戏进程。`log_tx` 为可选的实时日志接收端（每行输出投递一条 [`LogLine`]）。
pub fn spawn(command: &LaunchCommand, log_tx: Option<mpsc::Sender<LogLine>>) -> Result<GameSession> {
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.args);
    cmd.current_dir(&command.working_dir);
    // Java bin 目录进 PATH 头部：便于运行期定位与 java 同目录的运行时 DLL。
    if let Some(bin_dir) = command.program.parent() {
        cmd.env("PATH", prepend_path(bin_dir));
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // 不继承 stdin：游戏不读控制台输入，避免子进程挂在管道上。
    cmd.stdin(Stdio::null());

    let mut child = cmd.spawn().map_err(|source| LaunchError::Spawn {
        program: command.program.clone(),
        source,
    })?;

    let recent = Arc::new(Mutex::new(VecDeque::with_capacity(RECENT_LINE_CAPACITY)));
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(spawn_reader(stdout, LogStream::Stdout, recent.clone(), log_tx.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(spawn_reader(stderr, LogStream::Stderr, recent.clone(), log_tx));
    }

    Ok(GameSession {
        child,
        recent,
        readers,
    })
}

/// 崩溃触发判定：非零退出（或无退出码，多半是被杀/崩溃）即视为崩溃；即便退出码为 0，只要输出里出现明确的
/// 崩溃标记也判为崩溃（部分崩溃会正常退出但已生成崩溃报告）。
pub fn detect_crash(report: &ExitReport) -> bool {
    if crate::crash::has_crash_marker(&report.recent_lines.join("\n")) {
        return true;
    }
    !matches!(report.code, Some(0))
}

/// 起一个逐行读取任务：写入环形缓冲，并（若有）投递给实时消费者。
fn spawn_reader<R>(
    reader: R,
    stream: LogStream,
    recent: Arc<Mutex<VecDeque<String>>>,
    log_tx: Option<mpsc::Sender<LogLine>>,
) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        // 转发端可能中途被上层丢弃；此时停止转发但继续填充环形缓冲（崩溃分析仍需最后若干行）。
        let mut log_tx = log_tx;
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    push_recent(&recent, &line);
                    if let Some(tx) = &log_tx
                        && tx.send(LogLine { stream, text: line }).await.is_err()
                    {
                        log_tx = None;
                    }
                }
                Ok(None) => break, // 管道关闭（进程退出）。
                Err(err) => {
                    tracing::debug!(error = %err, "读取游戏进程输出出错，停止该流的读取");
                    break;
                }
            }
        }
    })
}

/// 把一行写入固定容量环形缓冲，超出容量丢最旧的。
fn push_recent(recent: &Arc<Mutex<VecDeque<String>>>, line: &str) {
    let mut buf = recent.lock().expect("环形缓冲锁未被毒化");
    if buf.len() == RECENT_LINE_CAPACITY {
        buf.pop_front();
    }
    buf.push_back(line.to_owned());
}

/// 取环形缓冲的当前快照。
fn recent_snapshot(recent: &Arc<Mutex<VecDeque<String>>>) -> Vec<String> {
    recent
        .lock()
        .expect("环形缓冲锁未被毒化")
        .iter()
        .cloned()
        .collect()
}

/// 把 `bin_dir` 拼到现有 PATH 头部。
fn prepend_path(bin_dir: &Path) -> std::ffi::OsString {
    let mut combined = bin_dir.as_os_str().to_owned();
    if let Some(existing) = std::env::var_os("PATH") {
        // Windows PATH 分隔符是 ';'（std::env::join_paths 会按平台处理，这里直接手拼头部一段）。
        let sep = if cfg!(windows) { ";" } else { ":" };
        combined.push(sep);
        combined.push(existing);
    }
    combined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_crash_by_exit_code() {
        let ok = ExitReport {
            code: Some(0),
            success: true,
            recent_lines: vec!["Stopping!".to_owned()],
        };
        assert!(!detect_crash(&ok));

        let nonzero = ExitReport {
            code: Some(1),
            success: false,
            recent_lines: vec!["some output".to_owned()],
        };
        assert!(detect_crash(&nonzero));

        let killed = ExitReport {
            code: None,
            success: false,
            recent_lines: vec![],
        };
        assert!(detect_crash(&killed));
    }

    #[test]
    fn detect_crash_by_marker_even_on_clean_exit() {
        let report = ExitReport {
            code: Some(0),
            success: true,
            recent_lines: vec!["---- Minecraft Crash Report ----".to_owned()],
        };
        assert!(detect_crash(&report));
    }

    #[test]
    fn ring_buffer_keeps_last_n_lines() {
        let recent = Arc::new(Mutex::new(VecDeque::with_capacity(RECENT_LINE_CAPACITY)));
        for i in 0..(RECENT_LINE_CAPACITY + 5) {
            push_recent(&recent, &format!("line {i}"));
        }
        let snapshot = recent_snapshot(&recent);
        assert_eq!(snapshot.len(), RECENT_LINE_CAPACITY);
        // 最旧的 5 行被挤掉，首行应为 "line 5"，末行为最后写入的那行。
        assert_eq!(snapshot.first().unwrap(), "line 5");
        assert_eq!(
            snapshot.last().unwrap(),
            &format!("line {}", RECENT_LINE_CAPACITY + 4)
        );
    }

    #[test]
    fn prepend_path_puts_bin_first() {
        // 无论现有 PATH 如何，结果都以 bin 目录开头。
        let combined = prepend_path(Path::new("C:/java/bin"));
        let s = combined.to_string_lossy();
        assert!(s.starts_with("C:/java/bin"));
    }

    // 真实 spawn 冒烟：Windows 上用 cmd 打印一行并以指定码退出，验证捕获与退出码回报。
    #[cfg(windows)]
    #[tokio::test]
    async fn spawn_captures_output_and_exit_code() {
        let command = LaunchCommand {
            program: std::path::PathBuf::from("cmd"),
            args: vec![
                "/C".to_owned(),
                "echo AURORA_HELLO & exit 3".to_owned(),
            ],
            working_dir: std::env::temp_dir(),
        };
        let (tx, mut rx) = mpsc::channel(16);
        let session = spawn(&command, Some(tx)).unwrap();
        let report = session.wait().await.unwrap();

        assert_eq!(report.code, Some(3));
        assert!(!report.success);
        assert!(
            report.recent_lines.iter().any(|l| l.contains("AURORA_HELLO")),
            "环形缓冲应含打印的行，实得 {:?}",
            report.recent_lines
        );

        // 实时通道也应至少收到那一行。
        let mut streamed = Vec::new();
        while let Ok(line) = rx.try_recv() {
            streamed.push(line.text);
        }
        assert!(streamed.iter().any(|l| l.contains("AURORA_HELLO")));
    }
}
