//! 游戏目录（`.minecraft`）的发现与管理。
//!
//! 一台机器上常常同时存在好几个 `.minecraft`：官方启动器的、PCL2 或 HMCL 的、以及 Aurora 自己的。
//! 这里把它们一并列出来，让玩家在其间切换，而不是逼他把攒了多年的存档搬到 Aurora 的目录下。
//!
//! 「当前目录」只有一个（[`AuroraConfig::game_directory`]，也是启动与安装真正落盘的地方），
//! 其余记在 [`AuroraConfig::extra_game_directories`] 里作为候选。切换目录就是把候选提为当前。

use std::path::{Path, PathBuf};

use aurora_instance::official_minecraft_dir;
use serde::Serialize;

use crate::config::NamedDirectory;
use crate::facade::Aurora;

/// 一条游戏目录记录。
///
/// 不复用 `aurora_instance::GameDirectory`：那个类型的语义是「扫描确认存在的目录」，
/// 而目录管理界面恰恰需要显示那些记着但当前不可达的条目（外置硬盘没插、盘符变了）。
/// 让记录凭空消失，玩家只会以为启动器把他的配置弄丢了。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameDirectoryEntry {
    pub name: String,
    pub path: PathBuf,
    /// 是否为当前正在使用的目录（安装与启动都落在它里面）。
    pub is_current: bool,
    /// 该目录此刻是否真实存在。为假时记录依然保留，插回硬盘就恢复。
    pub available: bool,
}

impl Aurora {
    /// 列出全部已知游戏目录：当前目录在前，其余按配置顺序。
    ///
    /// 只陈述配置里有什么，不去猜测磁盘上还有别的——探测新目录是
    /// [`Aurora::discover_game_directories`] 的事，两者分开是为了让「列出」这个高频操作
    /// 只做一次 `is_dir` 而不是满盘找。
    pub fn game_directories(&self) -> Vec<GameDirectoryEntry> {
        let current = self.game_dir().to_path_buf();
        let mut out = vec![GameDirectoryEntry {
            name: "当前文件夹".to_owned(),
            available: current.is_dir(),
            is_current: true,
            path: current.clone(),
        }];
        for extra in &self.config().extra_game_directories {
            // 当前目录同时也被记进额外列表时只留一条：它已经在最前面了。
            if same_path(&extra.path, &current) {
                continue;
            }
            out.push(GameDirectoryEntry {
                name: extra.name.clone(),
                available: extra.path.is_dir(),
                is_current: false,
                path: extra.path.clone(),
            });
        }
        out
    }

    /// 探测这台机器上可能存在的其它 `.minecraft`，返回尚未记录在配置里的那些。
    ///
    /// 初次设定用它把 PCL2、官方启动器的目录捞出来给玩家确认。只报告不写入：
    /// 是否收下由用户决定，静默把别人的游戏目录塞进自己的配置是越界。
    pub fn discover_game_directories(&self) -> Vec<NamedDirectory> {
        let mut found = Vec::new();
        let known: Vec<PathBuf> = std::iter::once(self.game_dir().to_path_buf())
            .chain(
                self.config()
                    .extra_game_directories
                    .iter()
                    .map(|d| d.path.clone()),
            )
            .collect();

        if let Some(official) = official_minecraft_dir()
            && official.is_dir()
            && !known.iter().any(|k| same_path(k, &official))
        {
            found.push(NamedDirectory {
                name: "官方启动器".to_owned(),
                path: official,
            });
        }

        for (name, path) in sibling_launcher_dirs() {
            if path.is_dir() && !known.iter().any(|k| same_path(k, &path)) && !found.iter().any(|f| same_path(&f.path, &path)) {
                found.push(NamedDirectory { name, path });
            }
        }
        found
    }

    /// 记下一个额外目录。同路径已存在时只更新名字，不产生重复条目。
    pub fn add_game_directory(&mut self, name: impl Into<String>, path: impl Into<PathBuf>) {
        let path = path.into();
        let name = name.into();
        let extras = &mut self.config_mut().extra_game_directories;
        match extras.iter_mut().find(|d| same_path(&d.path, &path)) {
            Some(existing) => existing.name = name,
            None => extras.push(NamedDirectory { name, path }),
        }
    }

    /// 移除一个额外目录；不存在返回 `false`。只从配置里摘掉，绝不动磁盘上的文件。
    pub fn remove_game_directory(&mut self, path: &Path) -> bool {
        let extras = &mut self.config_mut().extra_game_directories;
        let before = extras.len();
        extras.retain(|d| !same_path(&d.path, path));
        extras.len() != before
    }

    /// 把某个目录提为当前游戏目录。
    ///
    /// 原来的当前目录会被记进额外目录，否则切过去之后就再也找不回来了。
    pub fn switch_game_directory(&mut self, path: impl Into<PathBuf>, name: impl Into<String>) {
        let next = path.into();
        let previous = self.game_dir().to_path_buf();
        if same_path(&previous, &next) {
            return;
        }
        // 目标若在额外目录里，切过去之后它就是当前目录了，不该再重复列一遍。
        self.remove_game_directory(&next);
        let previous_name = previous
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "上一个目录".to_owned());
        self.add_game_directory(previous_name, previous);
        self.set_game_directory(next);
        let _ = name;
    }
}

/// 路径等价判定：Windows 下大小写不敏感，且要抹平 `\` 与 `/` 的混用。
///
/// 不走 `canonicalize`：那需要路径真实存在，而这里要比的恰恰包含「配置里记着但盘已经拔了」的条目。
fn same_path(a: &Path, b: &Path) -> bool {
    fn norm(p: &Path) -> String {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }
    norm(a) == norm(b)
}

/// 常见第三方启动器在各盘根下的 `.minecraft` 候选。
///
/// 只看固定的几个位置，不做全盘遍历：扫全盘既慢又会把玩家的整个磁盘翻一遍，
/// 找不到的让他自己添加即可。
fn sibling_launcher_dirs() -> Vec<(String, PathBuf)> {
    const LAUNCHER_DIRS: [(&str, &str); 3] = [
        ("PCL2", "Plain Craft Launcher 2"),
        ("PCL", "Plain Craft Launcher"),
        ("HMCL", "HMCL"),
    ];

    let mut out = Vec::new();
    for drive in ['C', 'D', 'E', 'F'] {
        let root = PathBuf::from(format!("{drive}:\\"));
        if !root.is_dir() {
            continue;
        }
        for (label, dir) in LAUNCHER_DIRS {
            let candidate = root.join(dir).join(".minecraft");
            if candidate.is_dir() {
                out.push((label.to_owned(), candidate));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;

    fn aurora_with(game_dir: &Path, extras: Vec<NamedDirectory>) -> Aurora {
        let config = AuroraConfig {
            extra_game_directories: extras,
            ..AuroraConfig::default()
        };
        Aurora::for_test(config, game_dir.to_path_buf(), game_dir.to_path_buf())
    }

    fn named(name: &str, path: &str) -> NamedDirectory {
        NamedDirectory {
            name: name.to_owned(),
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn same_path_ignores_case_separator_and_trailing_slash() {
        assert!(same_path(
            Path::new("E:/Games/.minecraft"),
            Path::new("e:\\games\\.minecraft\\")
        ));
        assert!(!same_path(
            Path::new("E:/Games/.minecraft"),
            Path::new("E:/Games/.minecraft2")
        ));
    }

    #[test]
    fn add_game_directory_updates_name_instead_of_duplicating() {
        let tmp = tempfile::tempdir().unwrap();
        let mut aurora = aurora_with(tmp.path(), Vec::new());

        aurora.add_game_directory("旧名", "E:\\Games\\.minecraft");
        // 同一个路径换个写法再加一次：应当只更新名字，不多出一条。
        aurora.add_game_directory("新名", "e:/games/.minecraft/");

        let extras = &aurora.config().extra_game_directories;
        assert_eq!(extras.len(), 1);
        assert_eq!(extras[0].name, "新名");
    }

    #[test]
    fn remove_game_directory_reports_whether_anything_went() {
        let tmp = tempfile::tempdir().unwrap();
        let mut aurora = aurora_with(tmp.path(), vec![named("PCL2", "E:\\PCL2\\.minecraft")]);

        assert!(aurora.remove_game_directory(Path::new("e:/pcl2/.minecraft")));
        assert!(aurora.config().extra_game_directories.is_empty());
        // 再删一次没有东西可删。
        assert!(!aurora.remove_game_directory(Path::new("E:\\PCL2\\.minecraft")));
    }

    /// 切换目录必须把原来的当前目录收进额外目录，否则切过去就找不回来了。
    #[test]
    fn switch_game_directory_keeps_the_previous_one_reachable() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("Aurora").join(".minecraft");
        let mut aurora = aurora_with(&current, vec![named("PCL2", "E:\\PCL2\\.minecraft")]);

        aurora.switch_game_directory("E:\\PCL2\\.minecraft", "PCL2");

        assert_eq!(aurora.game_dir(), Path::new("E:\\PCL2\\.minecraft"));
        let extras = &aurora.config().extra_game_directories;
        // 目标从额外目录里移除（它现在是当前），原当前目录被收进来。
        assert_eq!(extras.len(), 1);
        assert_eq!(extras[0].path, current);
    }

    #[test]
    fn switching_to_the_same_directory_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().to_path_buf();
        let mut aurora = aurora_with(&current, Vec::new());

        aurora.switch_game_directory(&current, "自己");

        assert_eq!(aurora.game_dir(), current);
        assert!(aurora.config().extra_game_directories.is_empty());
    }

    /// 当前目录排在最前且标记为 current；不可达的额外目录照样列出，只是 available 为假。
    /// 这条钉住「记录不因盘没挂而消失」——把 available 改成过滤条件，断言立刻挂。
    #[test]
    fn game_directories_keeps_unavailable_entries_visible() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_with(tmp.path(), vec![named("PCL2", "E:\\不存在的盘\\.minecraft")]);

        let listed = aurora.game_directories();
        assert_eq!(listed.len(), 2);

        assert_eq!(listed[0].path, tmp.path());
        assert!(listed[0].is_current);
        assert!(listed[0].available);

        assert_eq!(listed[1].name, "PCL2");
        assert!(!listed[1].is_current);
        assert!(!listed[1].available, "盘没挂时该条目仍要列出，只是标记为不可达");
    }

    /// 当前目录若同时被记进额外列表，只出现一次。
    #[test]
    fn game_directories_does_not_duplicate_the_current_one() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().to_path_buf();
        let aurora = aurora_with(
            &current,
            vec![NamedDirectory {
                name: "重复记录".to_owned(),
                path: current.clone(),
            }],
        );

        let listed = aurora.game_directories();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].is_current);
    }

    /// 已经记录过的目录不该再被当成「新发现」重复推荐。
    #[test]
    fn discover_skips_already_known_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let known = tmp.path().join("known");
        std::fs::create_dir_all(&known).unwrap();

        let aurora = aurora_with(
            tmp.path(),
            vec![NamedDirectory {
                name: "已知".to_owned(),
                path: known.clone(),
            }],
        );
        let found = aurora.discover_game_directories();
        assert!(!found.iter().any(|d| same_path(&d.path, &known)));
    }
}
