//! 本地已装模组管理：扫描 `mods/` 目录、读取 jar 内元数据、启用/禁用切换。
//!
//! 支持三种元数据格式：Fabric 的 `fabric.mod.json`（jar 根）、Forge 的 `META-INF/mods.toml`、
//! NeoForge 的 `META-INF/neoforge.mods.toml`。启禁状态以文件名 `.disabled` 后缀表达。
//! jar 即 zip，读取走 spawn_blocking 避免阻塞异步 worker。

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::ModLoader;

/// 元数据来源格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataFormat {
    /// Fabric 的 `fabric.mod.json`。
    Fabric,
    /// Forge 的 `META-INF/mods.toml`。
    ForgeToml,
    /// NeoForge 的 `META-INF/neoforge.mods.toml`。
    NeoForgeToml,
}

/// 从 jar 解析出的模组元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModMetadata {
    /// 模组 id。
    pub mod_id: String,
    /// 展示名。
    pub name: Option<String>,
    /// 版本号（Forge/NeoForge 可能是 `${file.jarVersion}` 占位符，原样保留）。
    pub version: Option<String>,
    /// 描述。
    pub description: Option<String>,
    /// 作者列表。
    pub authors: Vec<String>,
    /// 所属加载器。
    pub loader: ModLoader,
    /// 元数据来源格式。
    pub format: MetadataFormat,
}

/// 一个已装模组条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledMod {
    /// 磁盘路径。
    pub path: PathBuf,
    /// 磁盘上的文件名（可能带 `.disabled` 后缀）。
    pub file_name: String,
    /// 是否启用（无 `.disabled` 后缀）。
    pub enabled: bool,
    /// 解析出的元数据；无可识别描述文件或不是合法 jar 时为 `None`。
    pub metadata: Option<ModMetadata>,
}

const FABRIC_DESCRIPTOR: &str = "fabric.mod.json";
const FORGE_DESCRIPTOR: &str = "META-INF/mods.toml";
const NEOFORGE_DESCRIPTOR: &str = "META-INF/neoforge.mods.toml";
const DISABLED_SUFFIX: &str = ".disabled";

/// 文件名是否是（启用或禁用态的）模组 jar。
fn is_mod_file(name: &str) -> bool {
    let base = name.strip_suffix(DISABLED_SUFFIX).unwrap_or(name);
    base.ends_with(".jar")
}

/// 文件名是否为禁用态（带 `.disabled` 后缀）。
pub fn is_disabled(name: &str) -> bool {
    name.ends_with(DISABLED_SUFFIX)
}

/// 扫描 `mods/` 目录，返回所有模组条目（按文件名排序，结果稳定）。
///
/// 单个 jar 元数据解析失败不会中断整轮扫描：记 `warn` 日志并把该条以「无元数据」列出——一个损坏
/// 模组不应让整份列表瞎掉。目录本身不可读才向上冒泡。
pub async fn scan_mods_dir(dir: impl AsRef<Path>) -> Result<Vec<InstalledMod>> {
    let dir = dir.as_ref();
    let mut read_dir = tokio::fs::read_dir(dir).await.map_err(|source| {
        Error::Base(aurora_base::Error::Io {
            path: dir.to_owned(),
            source,
        })
    })?;

    let mut mods = Vec::new();
    while let Some(entry) = read_dir.next_entry().await.map_err(|source| {
        Error::Base(aurora_base::Error::Io {
            path: dir.to_owned(),
            source,
        })
    })? {
        let file_type = entry.file_type().await.map_err(|source| {
            Error::Base(aurora_base::Error::Io {
                path: entry.path(),
                source,
            })
        })?;
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !is_mod_file(&file_name) {
            continue;
        }
        let path = entry.path();
        let enabled = !is_disabled(&file_name);
        let metadata = match parse_mod_metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "解析模组元数据失败，按无元数据列出"
                );
                None
            }
        };
        mods.push(InstalledMod {
            path,
            file_name,
            enabled,
            metadata,
        });
    }

    mods.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(mods)
}

/// 解析单个 jar 的模组元数据。无可识别描述文件或非合法 zip 返回 `Ok(None)`；描述文件存在但内容
/// 损坏返回 `Err`（不掩盖）。
pub async fn parse_mod_metadata(path: impl AsRef<Path>) -> Result<Option<ModMetadata>> {
    let path = path.as_ref().to_owned();
    tokio::task::spawn_blocking(move || extract_metadata_blocking(&path))
        .await
        .map_err(|source| Error::Base(aurora_base::Error::HashTaskJoin(source)))?
}

fn extract_metadata_blocking(path: &Path) -> Result<Option<ModMetadata>> {
    let file = std::fs::File::open(path).map_err(|source| {
        Error::Base(aurora_base::Error::Io {
            path: path.to_owned(),
            source,
        })
    })?;
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(archive) => archive,
        // 不是合法 zip/jar（例如占位文件）：按「无元数据」处理，而非报错。
        Err(zip::result::ZipError::InvalidArchive(_)) => return Ok(None),
        Err(source) => {
            return Err(Error::Zip {
                path: path.to_owned(),
                source,
            });
        }
    };

    // 优先级：neoforge.mods.toml -> mods.toml -> fabric.mod.json。一个 jar 通常只含其一。
    if let Some(bytes) = read_entry(&mut archive, NEOFORGE_DESCRIPTOR, path)? {
        return parse_toml_metadata(
            &bytes,
            ModLoader::NeoForge,
            MetadataFormat::NeoForgeToml,
            path,
        );
    }
    if let Some(bytes) = read_entry(&mut archive, FORGE_DESCRIPTOR, path)? {
        return parse_toml_metadata(&bytes, ModLoader::Forge, MetadataFormat::ForgeToml, path);
    }
    if let Some(bytes) = read_entry(&mut archive, FABRIC_DESCRIPTOR, path)? {
        return parse_fabric_metadata(&bytes, path);
    }
    Ok(None)
}

/// 从 zip 里读取指定条目的全部字节；条目不存在返回 `Ok(None)`。
fn read_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    path: &Path,
) -> Result<Option<Vec<u8>>> {
    match archive.by_name(name) {
        Ok(mut entry) => {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|source| Error::Zip {
                path: path.to_owned(),
                source: zip::result::ZipError::Io(source),
            })?;
            Ok(Some(buf))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(source) => Err(Error::Zip {
            path: path.to_owned(),
            source,
        }),
    }
}

/// `fabric.mod.json` 里作者可为字符串或 `{ "name": ... }` 对象。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FabricAuthor {
    Name(String),
    Detailed { name: String },
}

#[derive(Debug, Deserialize)]
struct FabricModJson {
    id: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    authors: Vec<FabricAuthor>,
}

fn parse_fabric_metadata(bytes: &[u8], path: &Path) -> Result<Option<ModMetadata>> {
    let raw: FabricModJson = serde_json::from_slice(bytes).map_err(|source| Error::Json {
        context: format!("fabric.mod.json in {}", path.display()),
        source,
    })?;
    let authors = raw
        .authors
        .into_iter()
        .map(|author| match author {
            FabricAuthor::Name(name) => name,
            FabricAuthor::Detailed { name } => name,
        })
        .collect();
    Ok(Some(ModMetadata {
        mod_id: raw.id,
        name: raw.name,
        version: raw.version,
        description: raw.description,
        authors,
        loader: ModLoader::Fabric,
        format: MetadataFormat::Fabric,
    }))
}

#[derive(Debug, Deserialize)]
struct ModsToml {
    #[serde(default)]
    mods: Vec<ModsTomlEntry>,
    /// 顶层作者（部分模组把 authors 写在顶层而非 `[[mods]]` 内）。
    #[serde(default)]
    authors: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModsTomlEntry {
    #[serde(rename = "modId")]
    mod_id: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    authors: Option<String>,
}

fn parse_toml_metadata(
    bytes: &[u8],
    loader: ModLoader,
    format: MetadataFormat,
    path: &Path,
) -> Result<Option<ModMetadata>> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        Error::Base(aurora_base::Error::Io {
            path: path.to_owned(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "mods.toml 不是合法 UTF-8",
            ),
        })
    })?;
    let parsed: ModsToml = toml::from_str(text).map_err(|source| Error::Toml {
        context: format!("mods.toml in {}", path.display()),
        source,
    })?;

    // 取首个 [[mods]] 作为主模组。没有条目视为无可用元数据。
    let top_authors = parsed.authors;
    let Some(entry) = parsed.mods.into_iter().next() else {
        return Ok(None);
    };
    let authors = split_authors(entry.authors.or(top_authors));
    Ok(Some(ModMetadata {
        mod_id: entry.mod_id,
        name: entry.display_name,
        version: entry.version,
        description: entry.description,
        authors,
        loader,
        format,
    }))
}

/// 把 `"作者甲, 作者乙"` 拆成去空白、去空项的作者列表。
fn split_authors(raw: Option<String>) -> Vec<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(|author| author.trim().to_string())
            .filter(|author| !author.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

/// 切换模组启用/禁用状态，返回切换后的新路径。
///
/// 启用即去掉 `.disabled` 后缀，禁用即追加。若已处于目标状态则无操作返回原路径；若目标文件名已存在
/// （同名启用与禁用副本冲突）返回 [`Error::ModStateConflict`]，避免覆盖。
pub async fn set_mod_enabled(path: impl AsRef<Path>, enabled: bool) -> Result<PathBuf> {
    let path = path.as_ref();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            Error::Base(aurora_base::Error::Io {
                path: path.to_owned(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "路径缺少文件名"),
            })
        })?;

    let target_name = if enabled {
        name.strip_suffix(DISABLED_SUFFIX).unwrap_or(name).to_string()
    } else if is_disabled(name) {
        name.to_string()
    } else {
        format!("{name}{DISABLED_SUFFIX}")
    };

    // 已是目标状态：不动，返回原路径。
    if target_name == name {
        return Ok(path.to_owned());
    }

    let target_path = path.with_file_name(&target_name);
    let exists = tokio::fs::try_exists(&target_path).await.map_err(|source| {
        Error::Base(aurora_base::Error::Io {
            path: target_path.clone(),
            source,
        })
    })?;
    if exists {
        return Err(Error::ModStateConflict { path: target_path });
    }

    tokio::fs::rename(path, &target_path)
        .await
        .map_err(|source| {
            Error::Base(aurora_base::Error::Io {
                path: target_path.clone(),
                source,
            })
        })?;
    Ok(target_path)
}

/// 启用模组（去掉 `.disabled` 后缀）。
pub async fn enable_mod(path: impl AsRef<Path>) -> Result<PathBuf> {
    set_mod_enabled(path, true).await
}

/// 禁用模组（追加 `.disabled` 后缀）。
pub async fn disable_mod(path: impl AsRef<Path>) -> Result<PathBuf> {
    set_mod_enabled(path, false).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 用给定条目写一个（deflate 压缩的）jar，读时会走真正的解压路径。
    fn build_jar(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[tokio::test]
    async fn parses_fabric_mod_json() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("sodium.jar");
        // 原始 str + as_bytes：byte-string 字面量不允许非 ASCII，这里描述含中文。
        let descriptor = r#"{
            "schemaVersion": 1,
            "id": "sodium",
            "version": "0.5.3",
            "name": "Sodium",
            "description": "渲染优化",
            "authors": ["jellysquid3", {"name": "IMS"}]
        }"#
        .as_bytes();
        build_jar(&jar, &[("fabric.mod.json", descriptor)]);

        let meta = parse_mod_metadata(&jar).await.unwrap().expect("应有元数据");
        assert_eq!(meta.mod_id, "sodium");
        assert_eq!(meta.name.as_deref(), Some("Sodium"));
        assert_eq!(meta.version.as_deref(), Some("0.5.3"));
        assert_eq!(meta.authors, vec!["jellysquid3", "IMS"]);
        assert_eq!(meta.loader, ModLoader::Fabric);
        assert_eq!(meta.format, MetadataFormat::Fabric);
    }

    #[tokio::test]
    async fn parses_forge_mods_toml() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("jei.jar");
        let descriptor = r#"modLoader="javafml"
loaderVersion="[47,)"
license="MIT"

[[mods]]
modId="jei"
version="15.2.0.27"
displayName="Just Enough Items"
authors="mezz"
description="物品查看"
"#
        .as_bytes();
        build_jar(&jar, &[("META-INF/mods.toml", descriptor)]);

        let meta = parse_mod_metadata(&jar).await.unwrap().expect("应有元数据");
        assert_eq!(meta.mod_id, "jei");
        assert_eq!(meta.name.as_deref(), Some("Just Enough Items"));
        assert_eq!(meta.version.as_deref(), Some("15.2.0.27"));
        assert_eq!(meta.authors, vec!["mezz"]);
        assert_eq!(meta.loader, ModLoader::Forge);
        assert_eq!(meta.format, MetadataFormat::ForgeToml);
    }

    #[tokio::test]
    async fn parses_neoforge_mods_toml_with_multiple_authors() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("jade.jar");
        let descriptor = br#"modLoader="javafml"
loaderVersion="[1,)"
license="MIT"

[[mods]]
modId="jade"
version="11.6.0"
displayName="Jade"
authors="Snownee, TrainGuys"
"#;
        build_jar(&jar, &[("META-INF/neoforge.mods.toml", descriptor)]);

        let meta = parse_mod_metadata(&jar).await.unwrap().expect("应有元数据");
        assert_eq!(meta.mod_id, "jade");
        assert_eq!(meta.name.as_deref(), Some("Jade"));
        assert_eq!(meta.version.as_deref(), Some("11.6.0"));
        assert_eq!(meta.authors, vec!["Snownee", "TrainGuys"]);
        assert_eq!(meta.loader, ModLoader::NeoForge);
        assert_eq!(meta.format, MetadataFormat::NeoForgeToml);
    }

    #[tokio::test]
    async fn jar_without_descriptor_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("nolib.jar");
        build_jar(&jar, &[("com/example/Lib.class", b"not-metadata")]);
        assert!(parse_mod_metadata(&jar).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn non_zip_file_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("garbage.jar");
        tokio::fs::write(&jar, b"this is not a zip").await.unwrap();
        assert!(parse_mod_metadata(&jar).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn malformed_fabric_descriptor_errors() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("broken.jar");
        build_jar(&jar, &[("fabric.mod.json", b"{ this is not json ")]);
        let err = parse_mod_metadata(&jar).await.unwrap_err();
        assert!(matches!(err, Error::Json { .. }));
    }

    #[tokio::test]
    async fn scan_detects_enabled_and_disabled_and_skips_non_jar() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path();
        build_jar(
            &mods.join("sodium.jar"),
            &[("fabric.mod.json", br#"{"id":"sodium","name":"Sodium"}"#)],
        );
        build_jar(
            &mods.join("lithium.jar.disabled"),
            &[("fabric.mod.json", br#"{"id":"lithium","name":"Lithium"}"#)],
        );
        tokio::fs::write(mods.join("readme.txt"), b"hi").await.unwrap();

        let scanned = scan_mods_dir(mods).await.unwrap();
        assert_eq!(scanned.len(), 2);
        // 按文件名排序：lithium.jar.disabled 在 sodium.jar 前。
        assert_eq!(scanned[0].file_name, "lithium.jar.disabled");
        assert!(!scanned[0].enabled);
        assert_eq!(
            scanned[0].metadata.as_ref().unwrap().mod_id,
            "lithium"
        );
        assert_eq!(scanned[1].file_name, "sodium.jar");
        assert!(scanned[1].enabled);
    }

    #[tokio::test]
    async fn disable_then_enable_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("sodium.jar");
        build_jar(&jar, &[("fabric.mod.json", br#"{"id":"sodium"}"#)]);

        let disabled = disable_mod(&jar).await.unwrap();
        assert_eq!(disabled.file_name().unwrap(), "sodium.jar.disabled");
        assert!(!tokio::fs::try_exists(&jar).await.unwrap());
        assert!(tokio::fs::try_exists(&disabled).await.unwrap());

        let enabled = enable_mod(&disabled).await.unwrap();
        assert_eq!(enabled, jar);
        assert!(tokio::fs::try_exists(&jar).await.unwrap());
    }

    #[tokio::test]
    async fn enabling_already_enabled_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("sodium.jar");
        build_jar(&jar, &[("fabric.mod.json", br#"{"id":"sodium"}"#)]);
        let same = enable_mod(&jar).await.unwrap();
        assert_eq!(same, jar);
    }

    #[tokio::test]
    async fn disable_conflict_when_target_exists() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("sodium.jar");
        let disabled = dir.path().join("sodium.jar.disabled");
        build_jar(&jar, &[("fabric.mod.json", br#"{"id":"sodium"}"#)]);
        build_jar(&disabled, &[("fabric.mod.json", br#"{"id":"sodium"}"#)]);

        let err = disable_mod(&jar).await.unwrap_err();
        match err {
            Error::ModStateConflict { path } => {
                assert_eq!(path.file_name().unwrap(), "sodium.jar.disabled")
            }
            other => panic!("期望 ModStateConflict，得到 {other:?}"),
        }
        // 原文件仍在（未被覆盖）。
        assert!(tokio::fs::try_exists(&jar).await.unwrap());
    }
}
