//! 服务端当前版本指针与不可变清单模型。

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::error::{Error, Result};
use crate::path::SafeRelativePath;

/// 当前支持的清单与快照 schema。
pub const SCHEMA_VERSION: u32 = 1;

const POINTER_DOCUMENT: &str = "整合包版本指针";
const MANIFEST_DOCUMENT: &str = "整合包清单";

/// `/api/v1/pack/latest` 返回的当前版本指针。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackPointer {
    pub pack_id: String,
    pub version: String,
    pub manifest_url: String,
    pub released_at: String,
    #[serde(default)]
    pub note: Option<String>,
    pub min_launcher_version: String,
}

impl PackPointer {
    /// 从完整 JSON 文档严格解析并执行字段语义校验。
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let pointer: Self = serde_json::from_slice(bytes).map_err(|source| Error::Json {
            document: POINTER_DOCUMENT,
            source,
        })?;
        pointer.validate()?;
        Ok(pointer)
    }

    /// 字符串版本的严格解析入口。
    pub fn from_json_str(json: &str) -> Result<Self> {
        Self::from_json_slice(json.as_bytes())
    }

    /// 校验手工构造的指针也满足与网络解析相同的约束。
    pub fn validate(&self) -> Result<()> {
        validate_required_text(POINTER_DOCUMENT, "pack_id", &self.pack_id)?;
        validate_required_text(POINTER_DOCUMENT, "version", &self.version)?;
        validate_required_text(POINTER_DOCUMENT, "released_at", &self.released_at)?;
        validate_http_url(POINTER_DOCUMENT, "manifest_url", &self.manifest_url)?;
        validate_required_text(
            POINTER_DOCUMENT,
            "min_launcher_version",
            &self.min_launcher_version,
        )?;
        semver::Version::parse(&self.min_launcher_version).map_err(|source| {
            Error::InvalidField {
                document: POINTER_DOCUMENT,
                field: "min_launcher_version".to_owned(),
                reason: source.to_string(),
            }
        })?;
        Ok(())
    }
}

/// 不可变的整合包版本清单。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub schema: u32,
    pub pack_id: String,
    pub version: String,
    pub minecraft: String,
    pub loader: LoaderSpec,
    pub files: Vec<ManifestFile>,
}

impl PackManifest {
    /// 从完整 JSON 文档严格解析并执行全部安全约束。
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(|source| Error::Json {
            document: MANIFEST_DOCUMENT,
            source,
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// 字符串版本的严格解析入口。
    pub fn from_json_str(json: &str) -> Result<Self> {
        Self::from_json_slice(json.as_bytes())
    }

    /// 校验手工构造的清单，避免绕过反序列化安全边界。
    pub fn validate(&self) -> Result<()> {
        validate_schema(MANIFEST_DOCUMENT, self.schema)?;
        validate_required_text(MANIFEST_DOCUMENT, "pack_id", &self.pack_id)?;
        validate_required_text(MANIFEST_DOCUMENT, "version", &self.version)?;
        validate_required_text(MANIFEST_DOCUMENT, "minecraft", &self.minecraft)?;
        self.loader.validate()?;

        let mut paths = BTreeSet::new();
        for (index, file) in self.files.iter().enumerate() {
            file.validate(index)?;
            let key = file.path.comparison_key();
            if !paths.insert(key) {
                return Err(Error::DuplicatePath {
                    document: MANIFEST_DOCUMENT,
                    path: file.path.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// 清单指定的 Minecraft 加载器。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoaderSpec {
    pub kind: LoaderKind,
    pub version: String,
}

impl LoaderSpec {
    fn validate(&self) -> Result<()> {
        validate_required_text(MANIFEST_DOCUMENT, "loader.version", &self.version)
    }
}

/// Aurora 能表达的加载器种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
    LiteLoader,
    OptiFine,
}

/// 单个受管文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub path: SafeRelativePath,
    pub sha1: Sha1Digest,
    pub size: u64,
    pub policy: FilePolicy,
    pub urls: Vec<String>,
}

impl ManifestFile {
    fn validate(&self, index: usize) -> Result<()> {
        if self.urls.is_empty() {
            return Err(Error::InvalidField {
                document: MANIFEST_DOCUMENT,
                field: format!("files[{index}].urls"),
                reason: "至少需要一个下载地址".to_owned(),
            });
        }
        for (url_index, url) in self.urls.iter().enumerate() {
            validate_http_url(
                MANIFEST_DOCUMENT,
                &format!("files[{index}].urls[{url_index}]"),
                url,
            )?;
        }
        Ok(())
    }
}

/// 文件所有权策略。未出现在清单中的路径天然属于 ignore 域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilePolicy {
    Managed,
    Seeded,
    Optional,
}

/// 规范化为小写的 40 位 SHA-1 十六进制摘要。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha1Digest(String);

impl Sha1Digest {
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, Sha1ValidationError> {
        let value = value.into();
        if value.len() != 40 {
            return Err(Sha1ValidationError::Length {
                actual: value.len(),
            });
        }
        if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Sha1ValidationError::NonHex);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha1Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for Sha1Digest {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for Sha1Digest {
    type Error = Sha1ValidationError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for Sha1Digest {
    type Error = Sha1ValidationError;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Serialize for Sha1Digest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha1Digest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(de::Error::custom)
    }
}

/// SHA-1 文本格式错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Sha1ValidationError {
    #[error("SHA-1 必须恰好为 40 个 ASCII 十六进制字符，实际长度 {actual}")]
    Length { actual: usize },
    #[error("SHA-1 含非十六进制字符")]
    NonHex,
}

pub(crate) fn validate_schema(document: &'static str, schema: u32) -> Result<()> {
    if schema == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(Error::UnsupportedSchema {
            document,
            expected: SCHEMA_VERSION,
            actual: schema,
        })
    }
}

pub(crate) fn validate_required_text(
    document: &'static str,
    field: &str,
    value: &str,
) -> Result<()> {
    if value.trim().is_empty() {
        return Err(Error::InvalidField {
            document,
            field: field.to_owned(),
            reason: "不能为空".to_owned(),
        });
    }
    if value != value.trim() || value.chars().any(char::is_control) {
        return Err(Error::InvalidField {
            document,
            field: field.to_owned(),
            reason: "不能含首尾空白或控制字符".to_owned(),
        });
    }
    Ok(())
}

fn validate_http_url(document: &'static str, field: &str, value: &str) -> Result<()> {
    validate_required_text(document, field, value)?;
    let parsed = url::Url::parse(value).map_err(|source| Error::InvalidField {
        document,
        field: field.to_owned(),
        reason: source.to_string(),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(Error::InvalidField {
            document,
            field: field.to_owned(),
            reason: "必须是含主机名的 HTTP(S) 绝对地址".to_owned(),
        });
    }
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.fragment().is_some() {
        return Err(Error::InvalidField {
            document,
            field: field.to_owned(),
            reason: "不能含用户凭据或片段".to_owned(),
        });
    }
    Ok(())
}
