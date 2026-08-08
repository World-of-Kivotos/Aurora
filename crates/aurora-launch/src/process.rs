//! 游戏进程管理：spawn、stdout/stderr 流式捕获、退出码回报、崩溃触发判定。
//!
//! 用 `java.exe`（而非 `javaw.exe`，规避已知的输出重定向 Bug）启动，把 Java bin 目录塞进子进程 PATH 头部
//! （某些 natives 靠系统查找同目录 DLL），并把 stdout/stderr 逐行捕获：一路投递给上层的实时消费者
//! （用于日志窗口/加载进度估算），一路进一个固定容量的环形缓冲，供进程退出后的崩溃分析取用最后若干行，
//! 一路写进 [`crate::logfile`] 的会话归档，供进程结束很久之后回看现场。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::command::LaunchCommand;
use crate::error::{LaunchError, Result};
use crate::logfile::{LogArchiver, session_log_path};

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
    /// 是否由启动器主动终止（[`GameSession::kill`] 调用过）。主动终止的退出一律不算崩溃。
    #[serde(default)]
    pub terminated_by_launcher: bool,
}

/// 一次已启动的游戏会话。
pub struct GameSession {
    child: Child,
    recent: Arc<Mutex<VecDeque<String>>>,
    readers: Vec<JoinHandle<()>>,
    /// kill 过即置位，随后由 [`GameSession::wait`] 带进 [`ExitReport`]。
    terminated_by_launcher: bool,
    /// 本次会话的日志归档写入器；两个读取任务共享，归档打不开时为 `None`。
    archive: Option<Arc<AsyncMutex<LogArchiver>>>,
    /// 归档文件路径（归档打不开时为 `None`），供上层提供「打开日志」。
    archive_path: Option<PathBuf>,
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

    /// 本次会话的日志归档文件路径；归档未能建立时为 `None`。
    pub fn archive_path(&self) -> Option<&Path> {
        self.archive_path.as_deref()
    }

    /// 强制结束进程（对应「取消启动」/「结束游戏」）。
    ///
    /// 先置位再 kill：即便 kill 本身失败（进程已自行退出），这次意图也已记录，
    /// 后续 [`ExitReport`] 不会把这次退出误认成崩溃。
    pub async fn kill(&mut self) -> Result<()> {
        self.terminated_by_launcher = true;
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
        // 读取任务已全部结束，此处是把归档尾巴落盘的确定性时机。归档失败不得吞掉退出报告
        // （退出码与崩溃现场才是本函数的主产物），故只记警告。
        if let Some(archive) = &self.archive
            && let Err(err) = archive.lock().await.flush().await
        {
            tracing::warn!(error = %err, "游戏日志归档收尾 flush 失败");
        }
        Ok(ExitReport {
            code: status.code(),
            success: status.success(),
            recent_lines: recent_snapshot(&self.recent),
            terminated_by_launcher: self.terminated_by_launcher,
        })
    }
}

/// 启动游戏进程。`log_tx` 为可选的实时日志接收端（每行输出投递一条 [`LogLine`]）。
///
/// 输出同时归档到 `<工作目录>/.aurora/logs/<启动时刻>.log`（路径见 [`GameSession::archive_path`]）；
/// 归档独立于 `log_tx`，前端不订阅日志也照样留下现场。
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

    // 归档是辅助设施：磁盘满、目录只读之类的问题不该让玩家启动不了游戏，因此开不出归档时记一条警告
    // 降级放行，而不是把整次启动判失败。
    let archive_path = session_log_path(&command.working_dir, unix_now());
    let archive = match LogArchiver::create_blocking(&archive_path) {
        Ok(archiver) => Some(Arc::new(AsyncMutex::new(archiver))),
        Err(err) => {
            tracing::warn!(
                path = %archive_path.display(),
                error = %err,
                "游戏日志归档创建失败，本次会话不归档"
            );
            None
        }
    };

    let recent = Arc::new(Mutex::new(VecDeque::with_capacity(RECENT_LINE_CAPACITY)));
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(spawn_reader(
            stdout,
            LogStream::Stdout,
            recent.clone(),
            log_tx.clone(),
            archive.clone(),
        ));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(spawn_reader(
            stderr,
            LogStream::Stderr,
            recent.clone(),
            log_tx,
            archive.clone(),
        ));
    }

    Ok(GameSession {
        child,
        recent,
        readers,
        terminated_by_launcher: false,
        archive_path: archive.as_ref().map(|_| archive_path),
        archive,
    })
}

/// 崩溃触发判定：非零退出（或无退出码，多半是被杀/崩溃）即视为崩溃；即便退出码为 0，只要输出里出现明确的
/// 崩溃标记也判为崩溃（部分崩溃会正常退出但已生成崩溃报告）。
///
/// 启动器主动终止的会话先行短路：Windows 上 `TerminateProcess` 会留下非零退出码、Unix 上被信号杀死则
/// 连退出码都没有，两种表现都会命中下面的启发式——玩家点「结束游戏」不该收到一次崩溃指认。
pub fn detect_crash(report: &ExitReport) -> bool {
    if report.terminated_by_launcher {
        return false;
    }
    if crate::crash::has_crash_marker(&report.recent_lines.join("\n")) {
        return true;
    }
    !matches!(report.code, Some(0))
}

/// 起一个逐行读取任务：写入环形缓冲、写入会话归档，并（若有）投递给实时消费者。
fn spawn_reader<R>(
    reader: R,
    stream: LogStream,
    recent: Arc<Mutex<VecDeque<String>>>,
    log_tx: Option<mpsc::Sender<LogLine>>,
    archive: Option<Arc<AsyncMutex<LogArchiver>>>,
) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        // 转发端可能中途被上层丢弃；此时停止转发但继续填充环形缓冲（崩溃分析仍需最后若干行）。
        let mut log_tx = log_tx;
        // 归档写失败（磁盘满等）后本流不再重试：每行都撞同一个错只会刷屏，且日志已经不完整。
        let mut archive = archive;
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    push_recent(&recent, &line);
                    // 先归档再转发：归档写的是原始行（不加流别前缀，否则崩溃诊断的正则会失配），
                    // 而转发要把 line 移进 LogLine，顺序反过来就得多克隆一份字符串。
                    let mut archive_failed = false;
                    if let Some(handle) = archive.as_ref()
                        && let Err(err) = handle.lock().await.write_line(&line).await
                    {
                        tracing::warn!(
                            error = %err,
                            ?stream,
                            "游戏日志归档写入失败，本流停止归档"
                        );
                        archive_failed = true;
                    }
                    // handle 的借用到此结束，才能把 archive 置空。
                    if archive_failed {
                        archive = None;
                    }
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

/// 当前 Unix 秒，用作归档文件名。
///
/// 系统时钟早于纪元时退化为 0（归档落到 `0.log`，照常可写可读），不为一个文件名去打断启动。
fn unix_now() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs(),
        Err(_) => 0,
    }
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
            terminated_by_launcher: false,
        };
        assert!(!detect_crash(&ok));

        let nonzero = ExitReport {
            code: Some(1),
            success: false,
            recent_lines: vec!["some output".to_owned()],
            terminated_by_launcher: false,
        };
        assert!(detect_crash(&nonzero));

        // 外部原因被杀（非启动器所为）：无从判断，保守判为崩溃。
        let killed = ExitReport {
            code: None,
            success: false,
            recent_lines: vec![],
            terminated_by_launcher: false,
        };
        assert!(detect_crash(&killed));
    }

    #[test]
    fn detect_crash_by_marker_even_on_clean_exit() {
        let report = ExitReport {
            code: Some(0),
            success: true,
            recent_lines: vec!["---- Minecraft Crash Report ----".to_owned()],
            terminated_by_launcher: false,
        };
        assert!(detect_crash(&report));
    }

    /// 玩家点「结束游戏」：两个平台各自的退出表现都不得被当成崩溃。
    /// 删掉 detect_crash 里的主动终止短路，这两条断言即挂。
    #[test]
    fn launcher_termination_is_never_a_crash() {
        // Unix：被信号杀死，无退出码。
        let signaled = ExitReport {
            code: None,
            success: false,
            recent_lines: vec![],
            terminated_by_launcher: true,
        };
        assert!(!detect_crash(&signaled));

        // Windows：TerminateProcess 留下非零退出码。
        let terminated = ExitReport {
            code: Some(1),
            success: false,
            recent_lines: vec!["[Render thread] Stopping!".to_owned()],
            terminated_by_launcher: true,
        };
        assert!(!detect_crash(&terminated));
    }

    /// 主动终止优先于崩溃标记：玩家在游戏已经崩了之后点结束，仍按主动终止处理，
    /// 避免把一次用户操作报成崩溃。崩溃现场仍在 recent_lines 里，诊断可另行取用。
    #[test]
    fn launcher_termination_outranks_crash_marker() {
        let report = ExitReport {
            code: None,
            success: false,
            recent_lines: vec!["---- Minecraft Crash Report ----".to_owned()],
            terminated_by_launcher: true,
        };
        assert!(!detect_crash(&report));
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
    // 工作目录用临时目录而非系统 temp——spawn 现在会在工作目录下建 .aurora/logs/，不该污染 %TEMP% 根。
    #[cfg(windows)]
    #[tokio::test]
    async fn spawn_captures_output_and_exit_code() {
        let tmp = tempfile::tempdir().unwrap();
        let command = LaunchCommand {
            program: std::path::PathBuf::from("cmd"),
            args: vec![
                "/C".to_owned(),
                "echo AURORA_HELLO & exit 3".to_owned(),
            ],
            working_dir: tmp.path().to_path_buf(),
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

    /// 同一行既进实时通道也进归档文件；wait 返回时归档已落盘可直接读。
    /// 删掉 spawn_reader 里的归档写入，本测试即挂。
    #[cfg(windows)]
    #[tokio::test]
    async fn spawn_archives_every_line_to_the_session_log() {
        let tmp = tempfile::tempdir().unwrap();
        let command = LaunchCommand {
            program: std::path::PathBuf::from("cmd"),
            args: vec![
                "/C".to_owned(),
                "echo AURORA_ARCHIVED & echo AURORA_SECOND".to_owned(),
            ],
            working_dir: tmp.path().to_path_buf(),
        };
        let session = spawn(&command, None).unwrap();
        let archive_path = session
            .archive_path()
            .expect("可写工作目录下归档必须建立")
            .to_path_buf();
        assert_eq!(archive_path.parent().unwrap(), crate::logfile::log_dir(tmp.path()));

        let report = session.wait().await.unwrap();
        assert_eq!(report.code, Some(0));

        let archived = tokio::fs::read_to_string(&archive_path).await.unwrap();
        let lines: Vec<&str> = archived.lines().map(|l| l.trim()).collect();
        assert!(
            lines.contains(&"AURORA_ARCHIVED") && lines.contains(&"AURORA_SECOND"),
            "归档应逐行含两行输出，实得 {lines:?}"
        );

        // 归档目录里只有这一份会话日志。
        let logs = crate::logfile::list_archived_logs(tmp.path()).await.unwrap();
        assert_eq!(logs, vec![archive_path]);
    }
}
