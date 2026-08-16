//! 清单相对路径的跨平台安全边界。
//!
//! 清单统一使用正斜杠。类型负责拒绝 `..`、盘符、设备名与静态 Windows 路径别名；调用方
//! 在实际文件操作前仍须检查既有祖先中的符号链接和 reparse point。

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// 已通过词法安全校验的实例根相对路径。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SafeRelativePath(String);

impl SafeRelativePath {
    /// 校验并构造安全路径。
    pub fn new(path: impl Into<String>) -> std::result::Result<Self, PathValidationError> {
        let path = path.into();
        validate_relative_path(&path)?;
        Ok(Self(path))
    }

    /// 清单中的正斜杠路径。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 按组件拼到实例根目录；返回路径尚未通过磁盘别名检查。
    pub fn resolve_under(&self, root: &Path) -> PathBuf {
        self.0
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }

    /// Windows 文件系统比较键，用于拒绝仅大小写不同的别名路径。
    pub(crate) fn comparison_key(&self) -> String {
        self.0.to_lowercase()
    }
}

impl fmt::Display for SafeRelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for SafeRelativePath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for SafeRelativePath {
    type Error = PathValidationError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for SafeRelativePath {
    type Error = PathValidationError;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Serialize for SafeRelativePath {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SafeRelativePath {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(de::Error::custom)
    }
}

/// 路径被拒绝的精确原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathValidationError {
    #[error("路径不能为空")]
    Empty,
    #[error("路径必须相对实例根目录")]
    Absolute,
    #[error("路径必须使用正斜杠")]
    Backslash,
    #[error("路径含空组件")]
    EmptyComponent,
    #[error("路径含当前目录组件 '.'")]
    CurrentDirectory,
    #[error("路径含父目录组件 '..'")]
    ParentTraversal,
    #[error("路径含 Windows 盘符前缀")]
    DrivePrefix,
    #[error("路径进入客户端保护目录: {component}")]
    ProtectedTopLevel { component: String },
    #[error("路径组件以点或空格结尾: {component}")]
    TrailingDotOrSpace { component: String },
    #[error("路径组件含 Windows 非法字符: {component}")]
    InvalidCharacter { component: String },
    #[error("路径组件是 Windows 保留设备名: {component}")]
    ReservedName { component: String },
    #[error("路径组件疑似 DOS 短文件名别名: {component}")]
    DosShortName { component: String },
}

/// 校验字符串能否作为清单内的相对路径。
pub fn validate_relative_path(path: &str) -> std::result::Result<(), PathValidationError> {
    if path.is_empty() {
        return Err(PathValidationError::Empty);
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(PathValidationError::Absolute);
    }
    if path.contains('\\') {
        return Err(PathValidationError::Backslash);
    }
    if has_drive_prefix(path) {
        return Err(PathValidationError::DrivePrefix);
    }

    let first_component = path.split('/').next().unwrap_or(path);
    if is_protected_top_level(first_component) {
        return Err(PathValidationError::ProtectedTopLevel {
            component: first_component.to_owned(),
        });
    }

    for component in path.split('/') {
        validate_component(component)?;
    }
    Ok(())
}

fn validate_component(component: &str) -> std::result::Result<(), PathValidationError> {
    if component.is_empty() {
        return Err(PathValidationError::EmptyComponent);
    }
    if component == "." {
        return Err(PathValidationError::CurrentDirectory);
    }
    if component == ".." {
        return Err(PathValidationError::ParentTraversal);
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Err(PathValidationError::TrailingDotOrSpace {
            component: component.to_owned(),
        });
    }
    if is_dos_short_name(component) {
        return Err(PathValidationError::DosShortName {
            component: component.to_owned(),
        });
    }
    if component.chars().any(|character| {
        character <= '\u{1f}'
            || character == '\u{7f}'
            || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
    }) {
        return Err(PathValidationError::InvalidCharacter {
            component: component.to_owned(),
        });
    }
    if is_reserved_windows_name(component) {
        return Err(PathValidationError::ReservedName {
            component: component.to_owned(),
        });
    }
    Ok(())
}

fn has_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_protected_top_level(component: &str) -> bool {
    [".aurora", "saves", "screenshots", "logs"]
        .iter()
        .any(|protected| component.eq_ignore_ascii_case(protected))
}

fn is_dos_short_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let Some((prefix, suffix)) = stem.rsplit_once('~') else {
        return false;
    };
    !prefix.is_empty()
        && prefix.chars().count() <= 6
        && !prefix.contains('~')
        && !suffix.is_empty()
        && suffix.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_reserved_windows_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || device_number(&upper, "COM")
        || device_number(&upper, "LPT")
}

fn device_number(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| {
        matches!(
            suffix,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    })
}
