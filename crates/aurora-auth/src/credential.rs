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

    // 单一写入者（&mut AccountManager 串行化）+ 进程号后缀，避免残留临时文件互相覆盖。
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("credentials"),
        std::process::id()
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
}
