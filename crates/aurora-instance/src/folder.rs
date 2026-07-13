//! 多游戏目录（.minecraft）扫描、自定义列表与失效清理。
//!
//! 一个「游戏目录」是一份 `.minecraft`（内含 versions/、libraries/、assets/ 等）。启动器要能同时管理
//! 多个来源的游戏目录：启动器自身所在目录、官方启动器目录（`%APPDATA%\.minecraft`）、以及用户以
//! 「名称 > 路径」形式自定义的目录。扫描时剔除路径已失效的条目并去重；一个都没有时由调用方在回退
//! 位置自动创建一份新的 `.minecraft`。
//!
//! 这里的存在性/创建判定用同步 `std::fs`：都是针对少量目录的轻量 stat/mkdir，无需异步化。
//! 官方目录探测读 `%APPDATA%`，在非 Windows 平台自然得到 `None`（跨平台缝，本轮只落地 Windows 语义）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::MINECRAFT_DIR_NAME;
use crate::error::{Error, Result};

/// 一个游戏目录的来源。决定默认显示名与扫描优先级（Current > Official > Custom）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDirectorySource {
    /// 启动器自身目录下的 `.minecraft`。
    Current,
    /// 官方启动器目录（`%APPDATA%\.minecraft`）。
    Official,
    /// 用户自定义目录。
    Custom,
}

impl GameDirectorySource {
    /// 来源的默认中文显示名（Custom 用占位名，实际展示以用户命名为准）。
    pub fn default_name(self) -> &'static str {
        match self {
            GameDirectorySource::Current => "当前目录",
            GameDirectorySource::Official => "官方启动器",
            GameDirectorySource::Custom => "自定义",
        }
    }
}

/// 用户自定义的「名称 > 路径」条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomDirectory {
    pub name: String,
    pub path: PathBuf,
}

impl CustomDirectory {
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
        }
    }
}

/// 一份扫描确认存在的游戏目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDirectory {
    pub name: String,
    pub path: PathBuf,
    pub source: GameDirectorySource,
}

/// 游戏目录扫描器：持有三类来源（当前 / 官方 / 自定义列表），产出去重后的有效目录集合。
///
/// 当前目录与官方目录的具体路径由调用方注入（见 [`current_minecraft_dir`] / [`official_minecraft_dir`]），
/// 以便单元测试能给确定路径、绕开进程环境变量。
#[derive(Debug, Clone, Default)]
pub struct FolderScanner {
    current: Option<PathBuf>,
    official: Option<PathBuf>,
    custom: Vec<CustomDirectory>,
}

impl FolderScanner {
    /// 空扫描器（无任何来源）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置「当前目录」候选（应传入已定位到 `.minecraft` 的路径）。
    pub fn with_current(mut self, minecraft_dir: impl Into<PathBuf>) -> Self {
        self.current = Some(minecraft_dir.into());
        self
    }

    /// 设置「官方启动器」候选（应传入已定位到 `.minecraft` 的路径）。
    pub fn with_official(mut self, minecraft_dir: impl Into<PathBuf>) -> Self {
        self.official = Some(minecraft_dir.into());
        self
    }

    /// 设置自定义目录列表。
    pub fn with_custom(mut self, custom: Vec<CustomDirectory>) -> Self {
        self.custom = custom;
        self
    }

    /// 只读访问自定义列表。
    pub fn custom(&self) -> &[CustomDirectory] {
        &self.custom
    }

    /// 追加一个自定义目录。
    pub fn add_custom(&mut self, name: impl Into<String>, path: impl Into<PathBuf>) {
        self.custom.push(CustomDirectory::new(name, path));
    }

    /// 按路径移除自定义目录，返回是否移除了至少一条。
    pub fn remove_custom(&mut self, path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        let before = self.custom.len();
        self.custom.retain(|c| c.path != path);
        self.custom.len() != before
    }

    /// 清理路径已失效（不存在或不是目录）的自定义条目，返回被移除的条目列表。
    /// 对应「失效时清理设置」：只作用于自定义列表，不动当前/官方来源。
    pub fn cleanup_invalid(&mut self) -> Vec<CustomDirectory> {
        let mut removed = Vec::new();
        self.custom.retain(|c| {
            if c.path.is_dir() {
                true
            } else {
                removed.push(c.clone());
                false
            }
        });
        removed
    }

    /// 扫描出当前所有有效（真实存在的目录）的游戏目录，按来源优先级排序并按真实路径去重。
    /// 同一物理目录被多个来源命中时，保留优先级更高者（当前 > 官方 > 自定义）。
    pub fn scan(&self) -> Vec<GameDirectory> {
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut out = Vec::new();

        if let Some(path) = &self.current {
            push_if_dir(
                &mut out,
                &mut seen,
                GameDirectorySource::Current.default_name().to_string(),
                path.clone(),
                GameDirectorySource::Current,
            );
        }
        if let Some(path) = &self.official {
            push_if_dir(
                &mut out,
                &mut seen,
                GameDirectorySource::Official.default_name().to_string(),
                path.clone(),
                GameDirectorySource::Official,
            );
        }
        for custom in &self.custom {
            push_if_dir(
                &mut out,
                &mut seen,
                custom.name.clone(),
                custom.path.clone(),
                GameDirectorySource::Custom,
            );
        }
        out
    }

    /// 扫描；若一个可用目录都没有，则在 `fallback_minecraft_dir` 处创建一份新的 `.minecraft`
    /// 并作为唯一结果返回。对应「找不到任何可用文件夹时自动创建」。
    pub fn ensure_available(&self, fallback_minecraft_dir: &Path) -> Result<Vec<GameDirectory>> {
        let found = self.scan();
        if !found.is_empty() {
            return Ok(found);
        }
        std::fs::create_dir_all(fallback_minecraft_dir).map_err(|source| Error::Io {
            path: fallback_minecraft_dir.to_owned(),
            source,
        })?;
        Ok(vec![GameDirectory {
            name: GameDirectorySource::Current.default_name().to_string(),
            path: fallback_minecraft_dir.to_owned(),
            source: GameDirectorySource::Current,
        }])
    }
}

/// 把候选路径纳入结果：仅当它是真实存在的目录、且其规范化路径未出现过时才加入。
fn push_if_dir(
    out: &mut Vec<GameDirectory>,
    seen: &mut HashSet<PathBuf>,
    name: String,
    path: PathBuf,
    source: GameDirectorySource,
) {
    if !path.is_dir() {
        return;
    }
    // 规范化仅作去重键（Windows 上形如 \\?\C:\...），失败则退回原路径；结果里始终保留原始路径。
    let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if seen.insert(key) {
        out.push(GameDirectory { name, path, source });
    }
}

/// 官方启动器的 `.minecraft` 目录：`%APPDATA%\.minecraft`。缺少 APPDATA（含非 Windows）时返回 `None`。
pub fn official_minecraft_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join(MINECRAFT_DIR_NAME))
}

/// 启动器所在目录下的 `.minecraft` 候选路径。
pub fn current_minecraft_dir(launcher_root: impl AsRef<Path>) -> PathBuf {
    launcher_root.as_ref().join(MINECRAFT_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在临时根下建一个 .minecraft 目录并返回其路径。
    fn make_mc(root: &Path, name: &str) -> PathBuf {
        let p = root.join(name);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn scan_keeps_existing_drops_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let current = make_mc(tmp.path(), "current-mc");
        let missing = tmp.path().join("ghost-mc"); // 不创建

        let scanner = FolderScanner::new()
            .with_current(current.clone())
            .with_custom(vec![CustomDirectory::new("幽灵", missing)]);

        let found = scanner.scan();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, current);
        assert_eq!(found[0].source, GameDirectorySource::Current);
        assert_eq!(found[0].name, "当前目录");
    }

    #[test]
    fn scan_orders_current_official_custom_and_names_custom() {
        let tmp = tempfile::tempdir().unwrap();
        let current = make_mc(tmp.path(), "cur");
        let official = make_mc(tmp.path(), "off");
        let custom = make_mc(tmp.path(), "cus");

        let scanner = FolderScanner::new()
            .with_current(current.clone())
            .with_official(official.clone())
            .with_custom(vec![CustomDirectory::new("我的整合", custom.clone())]);

        let found = scanner.scan();
        assert_eq!(
            found.iter().map(|g| g.source).collect::<Vec<_>>(),
            vec![
                GameDirectorySource::Current,
                GameDirectorySource::Official,
                GameDirectorySource::Custom,
            ]
        );
        assert_eq!(found[2].name, "我的整合");
        assert_eq!(found[2].path, custom);
    }

    #[test]
    fn scan_dedups_same_physical_dir_keeping_higher_priority() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = make_mc(tmp.path(), "shared-mc");

        // 当前与自定义指向同一物理目录，应只保留一条且归给优先级更高的 Current。
        let scanner = FolderScanner::new()
            .with_current(shared.clone())
            .with_custom(vec![CustomDirectory::new("重复", shared.clone())]);

        let found = scanner.scan();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, GameDirectorySource::Current);
    }

    #[test]
    fn cleanup_invalid_removes_only_missing_custom() {
        let tmp = tempfile::tempdir().unwrap();
        let alive = make_mc(tmp.path(), "alive");
        let dead = tmp.path().join("dead");

        let mut scanner = FolderScanner::new().with_custom(vec![
            CustomDirectory::new("存活", alive.clone()),
            CustomDirectory::new("失效", dead.clone()),
        ]);

        let removed = scanner.cleanup_invalid();
        assert_eq!(removed, vec![CustomDirectory::new("失效", dead)]);
        assert_eq!(scanner.custom(), &[CustomDirectory::new("存活", alive)]);
    }

    #[test]
    fn remove_custom_by_path() {
        let mut scanner = FolderScanner::new().with_custom(vec![
            CustomDirectory::new("a", "D:\\a"),
            CustomDirectory::new("b", "D:\\b"),
        ]);
        assert!(scanner.remove_custom("D:\\a"));
        assert!(!scanner.remove_custom("D:\\a"));
        assert_eq!(scanner.custom().len(), 1);
        assert_eq!(scanner.custom()[0].name, "b");
    }

    #[test]
    fn ensure_available_creates_when_none() {
        let tmp = tempfile::tempdir().unwrap();
        let fallback = tmp.path().join("root").join(".minecraft");
        assert!(!fallback.exists());

        let scanner = FolderScanner::new(); // 无任何来源
        let dirs = scanner.ensure_available(&fallback).unwrap();

        assert!(fallback.is_dir());
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].path, fallback);
        assert_eq!(dirs[0].source, GameDirectorySource::Current);
    }

    #[test]
    fn ensure_available_does_not_create_when_scan_nonempty() {
        let tmp = tempfile::tempdir().unwrap();
        let current = make_mc(tmp.path(), "cur");
        let fallback = tmp.path().join("should-not-exist").join(".minecraft");

        let scanner = FolderScanner::new().with_current(current.clone());
        let dirs = scanner.ensure_available(&fallback).unwrap();

        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].path, current);
        assert!(!fallback.exists());
    }

    #[test]
    fn current_minecraft_dir_appends_dotminecraft() {
        let dir = current_minecraft_dir("D:\\Launcher");
        assert_eq!(dir, PathBuf::from("D:\\Launcher").join(".minecraft"));
    }
}
