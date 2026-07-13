//! natives 解压。
//!
//! 版本 JSON 里被判定为 native 的库（旧式带 `natives` 映射的 classifier 件，或 1.19+ 独立的
//! `natives-<os>` 条目）下载下来是 jar，启动前须把其中的动态库解压到版本隔离的 natives 目录。
//! 解压尊重库自带的 `extract.exclude` 前缀规则（Mojang 通常排除 `META-INF/`）。zip 读取是同步、
//! CPU/IO 混合操作，故整段解压丢进 `spawn_blocking`，不阻塞异步 worker。

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use aurora_version::{Library, RuntimeContext, VersionJson, select_libraries};

use crate::error::{Result, io_err, zip_err};
use crate::layout::GameLayout;
use crate::maven;

/// 无显式 exclude 时的默认排除前缀：签名/清单目录不属于运行时动态库。
const DEFAULT_EXCLUDES: &[&str] = &["META-INF/"];

/// 判断一个归档条目是否应被解压：目录一律跳过，命中任一 exclude 前缀也跳过。
fn should_extract(entry_name: &str, excludes: &[String]) -> bool {
    if entry_name.ends_with('/') {
        return false;
    }
    !excludes.iter().any(|prefix| entry_name.starts_with(prefix.as_str()))
}

/// 把版本里当前平台适用的全部 natives 解压到 `versions/<target_id>/<target_id>-natives`。
///
/// `target_id` 是 natives 的落点版本（原版安装即版本自身；加载器版本则复用其原版的 natives 目录）。
/// 返回解压出的文件总数。任一 native jar 缺失或损坏都会冒泡（natives 缺失会直接导致游戏崩溃，
/// 不容静默跳过）。
pub async fn extract_all_natives(
    version: &VersionJson,
    ctx: &RuntimeContext,
    layout: &GameLayout,
    target_id: &str,
) -> Result<u32> {
    // 先在异步上下文里把「哪些 jar、排除什么、解到哪」算清楚，再把纯同步解压交给阻塞线程池。
    let dest = layout.natives_dir(target_id);
    let mut jobs: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for lib in select_libraries(version, ctx) {
        if !lib.is_native() {
            continue;
        }
        let Some(rel) = native_jar_relpath(lib, ctx) else {
            continue;
        };
        jobs.push((layout.library_path(&rel), effective_excludes(lib)));
    }

    tokio::task::spawn_blocking(move || {
        let mut count = 0u32;
        for (jar, excludes) in &jobs {
            count += extract_jar(jar, &dest, excludes)?;
        }
        Ok(count)
    })
    .await
    .map_err(|join| {
        io_err(
            PathBuf::from("<natives>"),
            io::Error::other(format!("natives 解压任务异常终止: {join}")),
        )
    })?
}

/// 某 native 库当前平台对应 jar 的相对仓库路径。
fn native_jar_relpath(lib: &Library, ctx: &RuntimeContext) -> Option<String> {
    if lib.natives.is_some() {
        // 旧式：从 classifiers 里挑当前平台件。
        if let Some(artifact) = lib.native_artifact(ctx)
            && let Some(path) = &artifact.path
        {
            return Some(path.clone());
        }
        let classifier = lib.native_classifier(ctx)?;
        return maven::artifact_path(&format!("{}:{classifier}", lib.name));
    }
    // 新式：classifier 已在 name 里，直接用主件路径或坐标。
    if let Some(path) = lib
        .downloads
        .as_ref()
        .and_then(|d| d.artifact.as_ref())
        .and_then(|a| a.path.clone())
    {
        return Some(path);
    }
    maven::artifact_path(&lib.name)
}

/// 库的有效 exclude 前缀：优先用其 `extract.exclude`，未声明则用默认（排除 META-INF）。
fn effective_excludes(lib: &Library) -> Vec<String> {
    match &lib.extract {
        Some(extract) if !extract.exclude.is_empty() => extract.exclude.clone(),
        _ => DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect(),
    }
}

/// 同步解压单个 jar 里未被排除的条目到 `dest`，保留其内部相对结构（用 enclosed_name 防目录穿越）。
fn extract_jar(jar: &Path, dest: &Path, excludes: &[String]) -> Result<u32> {
    let file = File::open(jar).map_err(|source| io_err(jar, source))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|source| zip_err(jar, source))?;

    let mut count = 0u32;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|source| zip_err(jar, source))?;
        if entry.is_dir() || !should_extract(entry.name(), excludes) {
            continue;
        }
        // enclosed_name 拒绝绝对路径与 `..`，None 说明条目名不安全，跳过。
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let target = dest.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|source| io_err(parent, source))?;
        }
        let mut out = File::create(&target).map_err(|source| io_err(&target, source))?;
        io::copy(&mut entry, &mut out).map_err(|source| io_err(&target, source))?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_version::OsName;
    use std::io::Write;

    #[test]
    fn should_extract_skips_dirs_and_excluded_prefixes() {
        let excludes = vec!["META-INF/".to_string()];
        assert!(should_extract("lwjgl.dll", &excludes));
        assert!(should_extract("windows/x64/OpenAL.dll", &excludes));
        assert!(!should_extract("META-INF/MANIFEST.MF", &excludes));
        assert!(!should_extract("natives/", &excludes)); // 目录
    }

    #[test]
    fn should_extract_honors_custom_excludes() {
        let excludes = vec!["binary/".to_string(), "readme.txt".to_string()];
        assert!(!should_extract("binary/foo.so", &excludes));
        assert!(!should_extract("readme.txt", &excludes));
        assert!(should_extract("libglfw.so", &excludes));
    }

    #[test]
    fn effective_excludes_defaults_to_meta_inf() {
        let lib = Library {
            name: "g:a:1".into(),
            downloads: None,
            url: None,
            natives: None,
            rules: None,
            extract: None,
        };
        assert_eq!(effective_excludes(&lib), vec!["META-INF/".to_string()]);
    }

    /// 现场造一个含一个 dll、一个 META-INF 的 jar，验证解压只落 dll。
    fn build_test_jar(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("lwjgl.dll", opts).unwrap();
        writer.write_all(b"fake-dll-bytes").unwrap();
        writer.start_file("META-INF/MANIFEST.MF", opts).unwrap();
        writer.write_all(b"Manifest-Version: 1.0\n").unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn extract_jar_writes_only_non_excluded_files() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("natives.jar");
        build_test_jar(&jar);
        let dest = dir.path().join("out-natives");

        let count = extract_jar(&jar, &dest, &["META-INF/".to_string()]).unwrap();
        assert_eq!(count, 1, "只应解压 dll");
        assert_eq!(std::fs::read(dest.join("lwjgl.dll")).unwrap(), b"fake-dll-bytes");
        assert!(!dest.join("META-INF").exists(), "META-INF 应被排除");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn extract_all_natives_pulls_from_new_style_library() {
        let dir = tempfile::tempdir().unwrap();
        let layout = GameLayout::new(dir.path());
        // 造出库落点上的 native jar。
        let rel = "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar";
        let jar_path = layout.library_path(rel);
        std::fs::create_dir_all(jar_path.parent().unwrap()).unwrap();
        build_test_jar(&jar_path);

        let version = VersionJson::from_json_str(
            r#"{"id":"1.21","libraries":[
                {"name":"org.lwjgl:lwjgl:3.3.3:natives-windows",
                 "downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar","sha1":"x","size":1,"url":"u"}}}
            ]}"#,
        )
        .unwrap();
        let ctx = RuntimeContext::new(OsName::Windows, "x86_64", 64);

        let count = extract_all_natives(&version, &ctx, &layout, "1.21").await.unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            std::fs::read(layout.natives_dir("1.21").join("lwjgl.dll")).unwrap(),
            b"fake-dll-bytes"
        );
    }
}
