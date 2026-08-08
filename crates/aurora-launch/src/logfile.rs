//! 游戏日志归档：把一次会话的 stdout/stderr 逐行落到 `<工作目录>/.aurora/logs/<会话时间戳>.log`。
//!
//! 归档只写原始行、不加流别前缀——崩溃诊断（[`crate::crash::analyze`]）是拿正则在原始日志文本上匹配的，
//! 任何自造前缀都会让规则失配。写入走内部缓冲（攒够 [`FLUSH_THRESHOLD`] 字节落一次盘），避免每行一次
//! 系统调用把游戏拖慢；缓冲在达到阈值、显式 [`LogArchiver::flush`] 与 drop 三处落盘——崩溃现场往往就在
//! 最后几行，尾巴丢了等于没归档。
//!
//! [`list_archived_logs`] 只扫 `logs/` 这一层目录，绝不下探实例根目录：Prism 的 issue #1450 正是因为
//! 扫全实例目录，在带大整合包的实例上把界面卡死。

use std::path::{Path, PathBuf};

use aurora_instance::AURORA_META_DIR;
use tokio::io::AsyncWriteExt;

use crate::error::{LaunchError, Result};

/// 归档子目录名（位于 `.aurora/` 下）。
const LOGS_DIR: &str = "logs";

/// 归档文件扩展名。
const LOG_EXTENSION: &str = "log";

/// 缓冲达到该字节数即落盘。
const FLUSH_THRESHOLD: usize = 8 * 1024;

/// 该实例的日志归档目录：`<工作目录>/.aurora/logs/`。
///
/// 注意口径：跟的是「工作目录」而非「版本目录」。隔离关闭时工作目录就是 `.minecraft` 根，日志会与
/// 版本目录下的 ledger/history 分处两地——这是刻意的，日志属于「这次在哪跑的」，不属于版本身份。
pub fn log_dir(working_dir: &Path) -> PathBuf {
    working_dir.join(AURORA_META_DIR).join(LOGS_DIR)
}

/// 本次会话的日志文件路径。
///
/// 文件名直接用启动时刻的 Unix 秒：本 crate 依赖树里没有任何日期库，手写历法换算只会凭空多一处出错点；
/// 纯数字名同时让 [`list_archived_logs`] 可以直接按数值排序，无需回读每个文件的元数据。
pub fn session_log_path(working_dir: &Path, started_at_unix: u64) -> PathBuf {
    log_dir(working_dir).join(format!("{started_at_unix}.{LOG_EXTENSION}"))
}

/// 逐行追加写入器。内部带缓冲，drop 时把残余缓冲补落到盘上。
pub struct LogArchiver {
    /// drop 里没法 await，只能另开一个同步句柄补写残余缓冲，因此必须留存路径。
    path: PathBuf,
    file: tokio::fs::File,
    buffer: Vec<u8>,
}

impl LogArchiver {
    /// 打开（不存在则创建）归档文件，缺失的父目录一并创建。
    ///
    /// 以追加方式打开：同一路径被重复打开时接着写，绝不截断——归档文件一旦被截断，这次会话之前的
    /// 现场就永久没了。
    pub async fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| io_error(parent, source))?;
        }
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .map_err(|source| io_error(path, source))?;
        Ok(Self::from_file(path.to_path_buf(), file))
    }

    /// [`LogArchiver::create`] 的同步版本，供同步的 [`crate::process::spawn`] 在进程起来的当下就把
    /// 归档句柄接上读取任务。
    ///
    /// spawn 的签名（同步）已被上层调用方钉死，而归档句柄必须在建立读取任务之前就位；开一个文件的阻塞
    /// 开销与紧邻的子进程 spawn 相比可以忽略，故此处直接走 std 再转成 tokio 句柄。
    pub(crate) fn create_blocking(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|source| io_error(path, source))?;
        Ok(Self::from_file(
            path.to_path_buf(),
            tokio::fs::File::from_std(file),
        ))
    }

    fn from_file(path: PathBuf, file: tokio::fs::File) -> Self {
        Self {
            path,
            file,
            buffer: Vec::with_capacity(FLUSH_THRESHOLD),
        }
    }

    /// 归档文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 追加一行（自动补换行符）。缓冲攒够阈值时顺带落盘。
    pub async fn write_line(&mut self, line: &str) -> Result<()> {
        self.buffer.extend_from_slice(line.as_bytes());
        self.buffer.push(b'\n');
        if self.buffer.len() >= FLUSH_THRESHOLD {
            self.flush().await?;
        }
        Ok(())
    }

    /// 把缓冲落盘。
    pub async fn flush(&mut self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        // 无论成败都清缓冲：write_all 失败时可能已写进去半截，留着缓冲会在 drop 的补写里造成重复字节，
        // 让日志本身变得不可信。错误如实返回，由调用方决定是否停止归档。
        let written = self.file.write_all(&self.buffer).await;
        self.buffer.clear();
        written.map_err(|source| io_error(&self.path, source))?;
        self.file
            .flush()
            .await
            .map_err(|source| io_error(&self.path, source))
    }
}

impl Drop for LogArchiver {
    fn drop(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        // drop 里无法 await，改用同步追加句柄把残余字节补完。宁可在这里做一次阻塞小写，也不能丢掉
        // 最后几行——崩溃现场恰恰就在那里。失败只能记日志，drop 不许 panic、也没有返回值。
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(mut file) => {
                use std::io::Write;
                if let Err(err) = file.write_all(&self.buffer) {
                    tracing::warn!(
                        path = %self.path.display(),
                        error = %err,
                        "日志归档残余缓冲写入失败，本次会话末尾若干行未落盘"
                    );
                }
            }
            Err(err) => tracing::warn!(
                path = %self.path.display(),
                error = %err,
                "日志归档残余缓冲写入失败：归档文件无法重新打开"
            ),
        }
    }
}

/// 列出该实例的历史日志文件，按会话时间倒序。
///
/// 严格只读 `.aurora/logs/` 这一层：子目录不下探，非 `.log` 文件不收。归档目录不存在（从未启动过）
/// 时返回空列表，这是正常态而非错误。
pub async fn list_archived_logs(working_dir: &Path) -> Result<Vec<PathBuf>> {
    let dir = log_dir(working_dir);
    let mut read_dir = match tokio::fs::read_dir(&dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_error(&dir, source)),
    };

    let mut found: Vec<(u64, PathBuf)> = Vec::new();
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|source| io_error(&dir, source))?
    {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .await
            .map_err(|source| io_error(&path, source))?;
        // 目录一律跳过，不递归。
        if !file_type.is_file() || !has_log_extension(&path) {
            continue;
        }
        found.push((session_timestamp(&path), path));
    }

    // 时间倒序；时间戳相同（含无法解析而同为 0 的外来文件）时按文件名倒序，保证顺序稳定可复现。
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.file_name().cmp(&a.1.file_name())));
    Ok(found.into_iter().map(|(_, path)| path).collect())
}

/// 扩展名是否为 `.log`（大小写不敏感——Windows 上手工改名很容易写成 `.LOG`）。
fn has_log_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(LOG_EXTENSION))
}

/// 从文件名主干解析会话时间戳。非本启动器产出的日志（名字不是纯数字）取 0，排序时沉底但仍如实列出。
fn session_timestamp(path: &Path) -> u64 {
    match path.file_stem().and_then(|stem| stem.to_str()) {
        Some(stem) => stem.parse::<u64>().unwrap_or(0),
        None => 0,
    }
}

/// 归档 IO 失败统一带上出错路径。
///
/// [`LaunchError`] 目前没有归档专用变体，这里借道 [`aurora_instance::Error::Io`]（Display 为
/// 「文件 IO 失败: {path}」，语义相符），经 `#[from]` 落到 [`LaunchError::Instance`]。
fn io_error(path: &Path, source: std::io::Error) -> LaunchError {
    aurora_instance::Error::Io {
        path: path.to_path_buf(),
        source,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dir_sits_under_aurora_meta_dir() {
        let working = Path::new("D:/mc/.minecraft/versions/1.21");
        assert_eq!(log_dir(working), working.join(".aurora").join("logs"));
    }

    #[test]
    fn session_log_path_is_named_by_start_timestamp() {
        let working = Path::new("D:/mc/.minecraft");
        assert_eq!(
            session_log_path(working, 1_785_000_000),
            working.join(".aurora").join("logs").join("1785000000.log")
        );
        // 边界：时钟退化到纪元时仍产出合法文件名。
        assert_eq!(
            session_log_path(working, 0),
            working.join(".aurora").join("logs").join("0.log")
        );
    }

    #[tokio::test]
    async fn create_makes_missing_directories_and_round_trips_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = session_log_path(tmp.path(), 1_700_000_000);
        assert!(!path.parent().unwrap().exists(), "前置条件：归档目录尚不存在");

        let mut archiver = LogArchiver::create(&path).await.unwrap();
        archiver.write_line("[Render thread] Starting").await.unwrap();
        archiver.write_line("[Render thread] 中文行").await.unwrap();
        archiver.flush().await.unwrap();

        assert_eq!(archiver.path(), path);
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "[Render thread] Starting\n[Render thread] 中文行\n"
        );
    }

    /// 归档的尾巴不能丢：不显式 flush 直接 drop，残余缓冲也必须落盘。
    /// 删掉 Drop 实现，本测试即挂。
    #[tokio::test]
    async fn drop_flushes_pending_buffer() {
        let tmp = tempfile::tempdir().unwrap();
        let path = session_log_path(tmp.path(), 1_700_000_001);
        {
            let mut archiver = LogArchiver::create(&path).await.unwrap();
            archiver
                .write_line("---- Minecraft Crash Report ----")
                .await
                .unwrap();
        }
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "---- Minecraft Crash Report ----\n"
        );
    }

    /// 缓冲攒满阈值即自动落盘（边界：恰好写满 FLUSH_THRESHOLD 字节），无需显式 flush。
    #[tokio::test]
    async fn buffer_flushes_on_threshold() {
        let tmp = tempfile::tempdir().unwrap();
        let path = session_log_path(tmp.path(), 1_700_000_002);
        let mut archiver = LogArchiver::create(&path).await.unwrap();

        // 每行 255 字符 + 换行 = 256 字节，写满阈值需要的行数。
        let line = "x".repeat(255);
        let line_count = FLUSH_THRESHOLD / 256;
        for _ in 0..line_count {
            archiver.write_line(&line).await.unwrap();
        }

        let text = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(text.len(), FLUSH_THRESHOLD, "写满阈值的那一刻应已落盘");
        assert_eq!(text.lines().count(), line_count);
    }

    /// 阈值未满时不落盘：证明缓冲确实存在，而不是每行直写。
    #[tokio::test]
    async fn buffer_holds_until_threshold_or_flush() {
        let tmp = tempfile::tempdir().unwrap();
        let path = session_log_path(tmp.path(), 1_700_000_003);
        let mut archiver = LogArchiver::create(&path).await.unwrap();
        archiver.write_line("still buffered").await.unwrap();

        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "");
        archiver.flush().await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "still buffered\n"
        );
    }

    /// 重新打开同一归档追加而非截断。
    #[tokio::test]
    async fn reopening_appends_instead_of_truncating() {
        let tmp = tempfile::tempdir().unwrap();
        let path = session_log_path(tmp.path(), 1_700_000_004);

        let mut first = LogArchiver::create(&path).await.unwrap();
        first.write_line("first").await.unwrap();
        first.flush().await.unwrap();
        drop(first);

        let mut second = LogArchiver::create(&path).await.unwrap();
        second.write_line("second").await.unwrap();
        second.flush().await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "first\nsecond\n"
        );
    }

    /// 只列 logs 目录本层的 .log 文件，按时间倒序；子目录不下探，非日志文件不收。
    #[tokio::test]
    async fn list_only_returns_log_files_in_that_directory_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = log_dir(tmp.path());
        tokio::fs::create_dir_all(dir.join("nested")).await.unwrap();
        // 边界：目录名恰好带 .log 后缀，也不能被当成日志文件收进来。
        tokio::fs::create_dir_all(dir.join("400.log")).await.unwrap();
        tokio::fs::write(dir.join("100.log"), b"oldest").await.unwrap();
        tokio::fs::write(dir.join("300.log"), b"middle").await.unwrap();
        tokio::fs::write(dir.join("200.log"), b"older").await.unwrap();
        // 大小写不敏感：手工改名成 .LOG 仍算日志。
        tokio::fs::write(dir.join("500.LOG"), b"newest").await.unwrap();
        tokio::fs::write(dir.join("latest.txt"), b"not a log").await.unwrap();
        tokio::fs::write(dir.join("crash-2026.json"), b"not a log").await.unwrap();
        tokio::fs::write(dir.join("nested").join("999.log"), b"never recursed")
            .await
            .unwrap();

        let logs = list_archived_logs(tmp.path()).await.unwrap();
        assert_eq!(
            logs,
            vec![
                dir.join("500.LOG"),
                dir.join("300.log"),
                dir.join("200.log"),
                dir.join("100.log"),
            ]
        );
    }

    /// 从未启动过（归档目录不存在）是正常态，返回空列表而非报错。
    #[tokio::test]
    async fn list_returns_empty_when_log_dir_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(list_archived_logs(tmp.path()).await.unwrap().is_empty());
    }

    /// 外来日志（文件名不是时间戳）不参与时间排序，一律沉底，但仍如实列出。
    #[tokio::test]
    async fn foreign_named_logs_sink_to_the_bottom() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = log_dir(tmp.path());
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("latest.log"), b"manual").await.unwrap();
        tokio::fs::write(dir.join("debug.log"), b"manual").await.unwrap();
        tokio::fs::write(dir.join("42.log"), b"session").await.unwrap();

        let logs = list_archived_logs(tmp.path()).await.unwrap();
        assert_eq!(
            logs,
            vec![
                dir.join("42.log"),
                dir.join("latest.log"),
                dir.join("debug.log"),
            ]
        );
    }

    /// 归档目录是一个同名文件时不能静默当作「没有日志」，必须冒泡。
    #[tokio::test]
    async fn list_bubbles_unexpected_io_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = log_dir(tmp.path());
        tokio::fs::create_dir_all(dir.parent().unwrap()).await.unwrap();
        tokio::fs::write(&dir, "logs 被占成了同名文件").await.unwrap();

        let err = list_archived_logs(tmp.path()).await.unwrap_err();
        match err {
            LaunchError::Instance(aurora_instance::Error::Io { path, .. }) => {
                assert_eq!(path, dir);
            }
            other => panic!("期望带路径的 IO 错误，得到 {other:?}"),
        }
    }

    #[tokio::test]
    async fn blocking_create_writes_the_same_way() {
        let tmp = tempfile::tempdir().unwrap();
        let path = session_log_path(tmp.path(), 1_700_000_005);
        let mut archiver = LogArchiver::create_blocking(&path).unwrap();
        archiver.write_line("from blocking ctor").await.unwrap();
        archiver.flush().await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "from blocking ctor\n"
        );
    }
}
