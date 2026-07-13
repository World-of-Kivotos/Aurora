//! Java 安装探测：注册表 / 常见安装目录 / PATH 三路，产出 [`JavaInstallation`] 列表。
//!
//! 每条候选最终都要跑一次 `java -version` 才算数——只有能正常执行并解析出版本的才收录。
//! 三路探测只负责产出「候选可执行文件路径」，[`probe`] 负责把候选变成带版本信息的安装项。
//!
//! 注册表读取是 Windows 专属能力，用 `#[cfg(windows)]` 隔开；非 Windows 平台给一个返回空的
//! 桩，保证 crate 仍能在其它平台编译（architecture.md 一节的「跨平台缝」）。

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::version::parse_java_version_output;

/// Java 可执行文件名。Windows 下探测的是 `java.exe`。
#[cfg(windows)]
pub const JAVA_EXE: &str = "java.exe";
/// Java 可执行文件名（非 Windows）。
#[cfg(not(windows))]
pub const JAVA_EXE: &str = "java";

/// 一条探测来源，供 UI 展示与后续排序偏好（如偏好托管下载的运行时）参考。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectSource {
    /// 来自 Windows 注册表 JavaSoft/发行版键。
    Registry,
    /// 来自常见安装目录扫描。
    CommonDir,
    /// 来自 PATH 环境变量。
    Path,
    /// 由本启动器自动下载安装到数据目录的运行时。
    Managed,
}

/// 一个已探测并成功识别的 Java 安装。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaInstallation {
    /// java 可执行文件的绝对路径。
    pub path: PathBuf,
    /// 归一后的版本号。
    pub version: crate::version::JavaVersion,
    /// 是否 64 位。
    pub is_64bit: bool,
    /// 实现/厂商名。
    pub vendor: String,
    /// 探测来源。
    pub source: DetectSource,
}

/// 探测系统中全部可用 Java：注册表 + 常见目录 + PATH，去重后逐个 `java -version` 识别。
///
/// 单个候选执行/解析失败不会中断整体扫描（一个坏 Java 不该拖垮全部探测），只记 debug 日志跳过。
pub fn detect_all() -> Vec<JavaInstallation> {
    let mut candidates: Vec<(PathBuf, DetectSource)> = Vec::new();

    for home in registry_java_homes() {
        candidates.push((java_exe_in_home(&home), DetectSource::Registry));
    }
    for exe in common_dir_javas(&default_search_roots()) {
        candidates.push((exe, DetectSource::CommonDir));
    }
    for exe in path_javas(std::env::var_os("PATH").as_deref()) {
        candidates.push((exe, DetectSource::Path));
    }

    probe_candidates_with(candidates, run_java_version)
}

/// 对单个 java 可执行文件跑 `java -version` 并解析成 [`JavaInstallation`]。
pub fn probe(path: &Path, source: DetectSource) -> Result<JavaInstallation> {
    probe_with(path, source, &run_java_version)
}

// ---- 内部：候选 -> 安装项 ----

/// 用注入的 `runner` 把候选列表逐个识别成安装项，同时按规范化路径去重、跳过不存在的路径。
///
/// 抽出 `runner` 是为了让单测能注入固定输出，绕开真实的 `java -version` 子进程。
fn probe_candidates_with<F>(
    candidates: Vec<(PathBuf, DetectSource)>,
    runner: F,
) -> Vec<JavaInstallation>
where
    F: Fn(&Path) -> Result<String>,
{
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (exe, source) in candidates {
        if !exe.is_file() {
            continue;
        }
        if !seen.insert(canonical_key(&exe)) {
            continue;
        }
        match probe_with(&exe, source, &runner) {
            Ok(install) => out.push(install),
            Err(err) => {
                tracing::debug!(path = %exe.display(), error = %err, "跳过无法识别为 Java 的候选");
            }
        }
    }
    out
}

fn probe_with<F>(path: &Path, source: DetectSource, runner: &F) -> Result<JavaInstallation>
where
    F: Fn(&Path) -> Result<String>,
{
    let output = runner(path)?;
    let probed = parse_java_version_output(&output)?;
    Ok(JavaInstallation {
        path: path.to_path_buf(),
        version: probed.version,
        is_64bit: probed.is_64bit,
        vendor: probed.vendor,
        source,
    })
}

/// 实际执行 `java -version`，把 stderr（Java 主要往这里打）与 stdout 拼在一起返回。
fn run_java_version(path: &Path) -> Result<String> {
    let mut cmd = std::process::Command::new(path);
    cmd.arg("-version");
    configure_no_window(&mut cmd);
    let output = cmd.output().map_err(|source| Error::JavaExec {
        path: path.to_path_buf(),
        source,
    })?;
    let mut combined = String::from_utf8_lossy(&output.stderr).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    Ok(combined)
}

/// 探测时抑制子进程的控制台窗口，避免每识别一个 Java 就闪一下黑框。
#[cfg(windows)]
fn configure_no_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW，避免控制台子进程弹窗。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_no_window(_cmd: &mut std::process::Command) {}

/// 去重键：优先用规范化后的绝对路径，失败退回原路径；统一小写（Windows 路径大小写不敏感）。
fn canonical_key(exe: &Path) -> String {
    std::fs::canonicalize(exe)
        .unwrap_or_else(|_| exe.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

fn java_exe_in_home(home: &Path) -> PathBuf {
    home.join("bin").join(JAVA_EXE)
}

// ---- 三路探测：候选路径产出 ----

/// 从 Windows 注册表读取各发行版登记的 Java 安装根目录（JavaHome / InstallationPath）。
#[cfg(windows)]
fn registry_java_homes() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};

    // JavaSoft 是 Oracle/官方登记处；其余是常见发行版自建的登记键。
    const VENDOR_KEYS: &[&str] = &[
        "SOFTWARE\\JavaSoft\\Java Runtime Environment",
        "SOFTWARE\\JavaSoft\\Java Development Kit",
        "SOFTWARE\\JavaSoft\\JRE",
        "SOFTWARE\\JavaSoft\\JDK",
        "SOFTWARE\\Eclipse Adoptium\\JDK",
        "SOFTWARE\\Eclipse Adoptium\\JRE",
        "SOFTWARE\\Eclipse Foundation\\JDK",
        "SOFTWARE\\Microsoft\\JDK",
        "SOFTWARE\\Azul Systems\\Zulu",
        "SOFTWARE\\BellSoft\\Liberica",
        "SOFTWARE\\Amazon Corretto",
    ];

    let mut homes = Vec::new();
    // 64 位与 32 位两套注册表视图都扫，兼顾 32 位 JVM 登记在 WOW6432Node 的情况。
    for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        for base in VENDOR_KEYS {
            let Ok(key) = hklm.open_subkey_with_flags(base, KEY_READ | view) else {
                continue;
            };
            for name in key.enum_keys().flatten() {
                let Ok(sub) = key.open_subkey_with_flags(&name, KEY_READ | view) else {
                    continue;
                };
                let home = sub
                    .get_value::<String, _>("JavaHome")
                    .or_else(|_| sub.get_value::<String, _>("InstallationPath"));
                if let Ok(home) = home {
                    homes.push(PathBuf::from(home));
                }
            }
        }
    }
    homes
}

/// 非 Windows 平台没有注册表探测，返回空（跨平台缝）。
#[cfg(not(windows))]
fn registry_java_homes() -> Vec<PathBuf> {
    Vec::new()
}

/// 组装常见安装目录候选根：系统 JDK 安装位、各启动器/工具的 runtime 目录、本启动器数据目录。
fn default_search_roots() -> Vec<PathBuf> {
    let env_dir = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let mut roots = Vec::new();

    // 系统级 JDK 安装根（各发行版直接装在 Program Files 下）。
    for key in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        let Some(base) = env_dir(key) else { continue };
        for sub in [
            "Java",
            "Eclipse Adoptium",
            "Eclipse Foundation",
            "Microsoft",
            "Zulu",
            "BellSoft",
            "Amazon Corretto",
            "Semeru",
        ] {
            roots.push(base.join(sub));
        }
    }

    // 官方启动器与常见工具的 runtime 目录。
    if let Some(appdata) = env_dir("APPDATA") {
        roots.push(appdata.join(".minecraft").join("runtime"));
    }
    if let Some(profile) = env_dir("USERPROFILE") {
        // IntelliJ 下载的 JDK、scoop 安装的 java。
        roots.push(profile.join(".jdks"));
        roots.push(profile.join("scoop").join("apps"));
    }

    // 本启动器自动下载安装的运行时。
    if let Ok(data) = aurora_base::fs::data_dir() {
        roots.push(data.join("java-runtime"));
    }

    roots
}

/// 在每个候选根下有限深度递归，收集所有 `<dir>/bin/<java 可执行文件>`。
fn common_dir_javas(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in roots {
        collect_java_in_dir(root, MAX_SCAN_DEPTH, &mut found);
    }
    found
}

/// 目录扫描的最大递归深度：足够覆盖 `runtime/<component>/<platform>/<component>/bin` 这类深层布局，
/// 又不至于把 scoop/apps 之类的大目录翻穿。
const MAX_SCAN_DEPTH: usize = 5;

fn collect_java_in_dir(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    // 当前目录自身若是个 Java home（含 bin/java），先收下。
    let candidate = dir.join("bin").join(JAVA_EXE);
    if candidate.is_file() {
        out.push(candidate);
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        // 只钻真实子目录，跳过符号链接/重解析点，避免绕环与踩进 javapath 转发目录。
        if file_type.is_dir() && !file_type.is_symlink() {
            collect_java_in_dir(&entry.path(), depth - 1, out);
        }
    }
}

/// 从 PATH 环境变量各目录下取 `java` 可执行文件。
fn path_javas(path_var: Option<&OsStr>) -> Vec<PathBuf> {
    let Some(var) = path_var else {
        return Vec::new();
    };
    std::env::split_paths(var)
        .map(|dir| dir.join(JAVA_EXE))
        .filter(|exe| exe.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个「运行器」：无论传什么路径都返回固定的 java -version 文本。
    fn fixed_runner(output: &'static str) -> impl Fn(&Path) -> Result<String> {
        move |_path: &Path| Ok(output.to_owned())
    }

    const OPENJDK_17: &str = "openjdk version \"17.0.1\" 2021-10-19\n\
        OpenJDK 64-Bit Server VM (build 17.0.1+12-39, mixed mode)\n";

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"stub").unwrap();
    }

    #[test]
    fn probe_with_assembles_installation_fields() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(JAVA_EXE);
        touch(&exe);

        let install =
            probe_with(&exe, DetectSource::Path, &fixed_runner(OPENJDK_17)).unwrap();
        assert_eq!(install.path, exe);
        assert_eq!(install.version.major, 17);
        assert!(install.is_64bit);
        assert_eq!(install.vendor, "OpenJDK");
        assert_eq!(install.source, DetectSource::Path);
    }

    #[test]
    fn candidates_are_deduplicated_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(JAVA_EXE);
        touch(&exe);

        // 同一路径出现两次，只应产出一条。
        let candidates = vec![
            (exe.clone(), DetectSource::Registry),
            (exe.clone(), DetectSource::Path),
        ];
        let found = probe_candidates_with(candidates, fixed_runner(OPENJDK_17));
        assert_eq!(found.len(), 1);
        // 去重保留首个来源（注册表）。
        assert_eq!(found[0].source, DetectSource::Registry);
    }

    #[test]
    fn nonexistent_candidates_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope").join(JAVA_EXE);
        let found =
            probe_candidates_with(vec![(missing, DetectSource::Path)], fixed_runner(OPENJDK_17));
        assert!(found.is_empty());
    }

    #[test]
    fn unrecognized_java_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good").join(JAVA_EXE);
        let bad = dir.path().join("bad").join(JAVA_EXE);
        touch(&good);
        touch(&bad);

        // 运行器按路径分叉：good 给正常输出，bad 给垃圾输出。
        let good_dir = good.parent().unwrap().to_path_buf();
        let runner = move |p: &Path| -> Result<String> {
            if p.starts_with(&good_dir) {
                Ok(OPENJDK_17.to_owned())
            } else {
                Ok("garbage not a java".to_owned())
            }
        };
        let found = probe_candidates_with(
            vec![
                (good.clone(), DetectSource::CommonDir),
                (bad, DetectSource::CommonDir),
            ],
            runner,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, good);
    }

    #[test]
    fn common_dir_scan_finds_nested_java() {
        // 造 <root>/jdk-17.0.1/bin/java(.exe) 与更深一层的 runtime 布局。
        let root = tempfile::tempdir().unwrap();
        let shallow = root.path().join("jdk-17.0.1").join("bin").join(JAVA_EXE);
        touch(&shallow);
        let deep = root
            .path()
            .join("runtime")
            .join("java-runtime-gamma")
            .join("windows-x64")
            .join("bin")
            .join(JAVA_EXE);
        touch(&deep);

        let found = common_dir_javas(&[root.path().to_path_buf()]);
        assert!(found.contains(&shallow), "应发现浅层 JDK");
        assert!(found.contains(&deep), "应发现深层 runtime 内的 java");
    }

    #[test]
    fn path_javas_reads_split_paths() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let exe_a = dir_a.path().join(JAVA_EXE);
        touch(&exe_a);
        // dir_b 下没有 java，应被过滤。

        let joined = std::env::join_paths([dir_a.path(), dir_b.path()]).unwrap();
        let found = path_javas(Some(joined.as_os_str()));
        assert_eq!(found, vec![exe_a]);
    }

    #[test]
    fn path_javas_none_is_empty() {
        assert!(path_javas(None).is_empty());
    }

    #[test]
    fn installation_carries_parsed_version() {
        // 冒烟：确保 JavaInstallation 的 version 就是从同一份输出解析出的那个（供 select 用）。
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(JAVA_EXE);
        touch(&exe);
        let install = probe_with(&exe, DetectSource::Managed, &fixed_runner(OPENJDK_17)).unwrap();
        let expected = parse_java_version_output(OPENJDK_17).unwrap().version;
        assert_eq!(install.version, expected);
        assert_eq!(install.version.major, 17);
    }
}
