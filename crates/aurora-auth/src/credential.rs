//! 凭据存储抽象。
//!
//! [`CredentialStore`] 只负责把“整份凭据明文字节”安全落盘与读回；序列化/反序列化由
//! [`crate::account::AccountManager`] 负责。这样加密后端（Windows DPAPI）可与账户逻辑解耦，
//! 也便于测试注入不加密的内存实现。
//!
//! 采用同步接口：凭据文件极小且写入不频繁，同步 IO 的阻塞可忽略，换来 `dyn` 兼容与更简单的调用方。

use std::io::Write;
use std::path::Path;

use crate::error::Result;

/// 凭据明文字节的持久化后端。实现内部自行决定是否加密。
pub trait CredentialStore {
    /// 读回此前保存的明文字节；从未保存过时返回 `Ok(None)`。
    fn load(&self) -> Result<Option<Vec<u8>>>;
    /// 持久化明文字节（实现内部负责加密与原子落盘）。
    fn save(&self, plaintext: &[u8]) -> Result<()>;
}

/// 同目录临时文件 + fsync + rename 的原子写入（跨平台，供文件型 store 复用）。
///
/// 与 aurora-base 的异步 `atomic_write` 同构，但走同步 std::fs 以匹配同步的 [`CredentialStore`]。
/// Windows 上 `std::fs::rename` 走 `MoveFileExW + REPLACE_EXISTING`，可覆盖已存在目标。
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> aurora_base::Result<()> {
    let parent = path.parent().ok_or_else(|| aurora_base::Error::Io {
        path: path.to_owned(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "凭据路径没有父目录，无法原子写入",
        ),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| aurora_base::Error::Io {
        path: parent.to_owned(),
        source,
    })?;

    // 进程号 + 进程内自增序号：两者缺一不可。「同一进程里只有一个写入者」这条旧前提已不成立——
    // 账户库与离线库都改成了每次操作现开一个实例（AccountManager / OfflineAccountStore），
    // 上层命令又只持共享锁，于是两次操作可能同时写同一个目标文件；临时名只挂进程号的话它们会踩进
    // 同一个临时文件，把彼此写了一半的内容 rename 出去，落盘结果既非旧值也非新值。
    // 与 aurora-base 那份异步 `atomic_write` 的随机后缀同一目的，此处不引入随机数依赖，用原子计数器。
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("credentials"),
        std::process::id(),
        TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));

    if let Err(err) = write_and_sync(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(source) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(aurora_base::Error::Io {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

fn write_and_sync(tmp: &Path, bytes: &[u8]) -> aurora_base::Result<()> {
    let mut file = std::fs::File::create(tmp).map_err(|source| aurora_base::Error::Io {
        path: tmp.to_owned(),
        source,
    })?;
    file.write_all(bytes).map_err(|source| aurora_base::Error::Io {
        path: tmp.to_owned(),
        source,
    })?;
    // fsync：确保 rename 前数据已落盘，避免掉电后目标名指向空洞文件。
    file.sync_all().map_err(|source| aurora_base::Error::Io {
        path: tmp.to_owned(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// 测试用不加密内存 store：验证 [`crate::account::AccountManager`] 的序列化与增删改查逻辑。
    ///
    /// 内部字节以 `Arc` 共享，`clone` 出的句柄指向同一份数据，可模拟“重启进程后从同一文件重载”。
    #[derive(Default, Clone)]
    pub struct InMemoryStore {
        bytes: Arc<Mutex<Option<Vec<u8>>>>,
    }

    impl InMemoryStore {
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl CredentialStore for InMemoryStore {
        fn load(&self) -> Result<Option<Vec<u8>>> {
            Ok(self.bytes.lock().expect("内存 store 锁").clone())
        }
        fn save(&self, plaintext: &[u8]) -> Result<()> {
            *self.bytes.lock().expect("内存 store 锁") = Some(plaintext.to_vec());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_creates_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join("credentials.bin");
        write_atomic(&file, b"first").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"first");

        write_atomic(&file, b"second-and-longer").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"second-and-longer");

        // 临时文件不应残留：目录内只剩目标文件。
        let names: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["credentials.bin".to_string()]);
    }

    /// 同一进程内并发写同一个目标文件：每次读回的都必须是某一位写入者的完整负载。
    ///
    /// 账户库与离线库都改成了每次操作现开一个实例、上层只持共享锁，两次账户操作因此可能真的
    /// 同时落到这里。临时名若只挂进程号，两个写入者就共用同一个临时文件：一方 create 截断、
    /// 另一方正写到一半，rename 出去的内容既非甲也非乙。把 tmp 名里的自增序号去掉，本用例即挂。
    #[test]
    fn concurrent_write_atomic_never_publishes_a_torn_file() {
        // 长度与填充字节都随写入者不同：任何截断或交错都会掉出这个合法集合。
        let payload = |writer: u8| vec![b'a' + writer; 4096 * (writer as usize + 1)];

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("credentials.bin");
        std::thread::scope(|scope| {
            for writer in 0..8u8 {
                let file = file.clone();
                scope.spawn(move || {
                    let mine = payload(writer);
                    for _ in 0..30 {
                        write_atomic(&file, &mine).unwrap();
                        let seen = std::fs::read(&file).unwrap();
                        let author = seen[0];
                        assert!(
                            (b'a'..b'a' + 8).contains(&author),
                            "读回的首字节不属于任何写入者：{author}"
                        );
                        assert_eq!(
                            seen,
                            payload(author - b'a'),
                            "读到了被撕裂的文件：长度或填充与任何一位写入者的完整负载都对不上"
                        );
                    }
                });
            }
        });
    }
}
