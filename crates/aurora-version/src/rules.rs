//! library / argument 的 rules 求值。
//!
//! Minecraft 版本 JSON 用 `rules` 数组按当前运行环境（os.name / os.version / os.arch / features）
//! 决定某个库或某段参数是否生效。求值语义严格对齐官方启动器：
//! 空规则视为放行；否则按顺序遍历，"最后一条命中的规则" 的 action 决定最终结果（默认不放行）。
//! 这条 "后者覆盖前者" 的语义是 `[{allow}, {disallow os=osx}]` 这类模式能在 osx 上被排除、
//! 在其它系统放行的关键。

use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// 规则动作：命中后是放行还是排除。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Disallow,
}

/// 规则中的操作系统约束。三个字段都是可选，缺省表示 "该维度不设限"。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OsRule {
    /// 目标系统名：windows / osx / linux。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 目标系统版本约束，值是一个正则（如 `^10\\.`），对运行环境的系统版本串做匹配。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// 目标架构：x86 / x86_64 / arm64 等。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
}

/// 单条规则：动作 + 可选的 os 约束 + 可选的 features 约束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub action: RuleAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
    /// 特性开关约束，如 `{"is_demo_user": true}`；键在运行环境里必须取到与之相等的布尔值才算命中。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<BTreeMap<String, bool>>,
}

impl Rule {
    /// 判断本条规则在给定运行环境下是否 "命中"（其全部约束都被满足）。
    ///
    /// 命中与否只决定这条规则要不要参与最终 action 计算，不直接等于放行。
    pub fn matches(&self, ctx: &RuntimeContext) -> bool {
        if let Some(os) = &self.os {
            if let Some(name) = &os.name
                && name != ctx.os_name.as_mojang()
            {
                return false;
            }
            if let Some(arch) = &os.arch
                && arch != &ctx.os_arch
            {
                return false;
            }
            if let Some(version) = &os.version {
                // 非法正则不应让整个求值 panic：视为该约束不满足，规则不命中。
                match Regex::new(version) {
                    Ok(re) if re.is_match(&ctx.os_version) => {}
                    _ => return false,
                }
            }
        }
        if let Some(features) = &self.features {
            for (feature, expected) in features {
                let actual = ctx.features.get(feature).copied().unwrap_or(false);
                if actual != *expected {
                    return false;
                }
            }
        }
        true
    }
}

/// 对一组规则求值，返回该库/参数在当前环境下是否生效。
///
/// 空规则集放行；否则以 "不放行" 为初值，命中的规则依次覆盖结果，最后一条命中的规则说了算。
pub fn evaluate_rules(rules: &[Rule], ctx: &RuntimeContext) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule.matches(ctx) {
            allowed = matches!(rule.action, RuleAction::Allow);
        }
    }
    allowed
}

/// 规则里出现的操作系统枚举，对应 JSON 中的 os.name 取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsName {
    Windows,
    Osx,
    Linux,
}

impl OsName {
    /// 返回 Mojang JSON 使用的系统名字符串。
    pub fn as_mojang(self) -> &'static str {
        match self {
            OsName::Windows => "windows",
            OsName::Osx => "osx",
            OsName::Linux => "linux",
        }
    }

    /// 从当前编译目标推断系统。非 win/mac/linux 目标返回 None。
    pub fn current() -> Option<Self> {
        match std::env::consts::OS {
            "windows" => Some(OsName::Windows),
            "macos" => Some(OsName::Osx),
            "linux" => Some(OsName::Linux),
            _ => None,
        }
    }
}

/// 规则求值所需的运行环境快照。由启动层填充真实值，解析层的测试可直接构造。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContext {
    /// 操作系统。
    pub os_name: OsName,
    /// 系统版本串，仅用于满足 os.version 正则约束；无约束场景可留空。
    pub os_version: String,
    /// 架构串，用于匹配 os.arch（x86 / x86_64 / arm64 / arm32 ...）。
    pub os_arch: String,
    /// 位数（32 / 64），用于把 natives 分类器里的 `${arch}` 占位符替换成具体值。
    pub arch_bits: u8,
    /// 已启用的特性开关集合。
    pub features: BTreeMap<String, bool>,
}

impl RuntimeContext {
    /// 用系统、架构串、位数构造一个不含任何特性开关的环境。
    pub fn new(os_name: OsName, os_arch: impl Into<String>, arch_bits: u8) -> Self {
        RuntimeContext {
            os_name,
            os_version: String::new(),
            os_arch: os_arch.into(),
            arch_bits,
            features: BTreeMap::new(),
        }
    }

    /// 从当前进程的编译目标推断运行环境。本项目只针对 Windows，其它系统探测失败时退回 Windows。
    pub fn current() -> Self {
        let os_name = OsName::current().unwrap_or(OsName::Windows);
        let os_arch = match std::env::consts::ARCH {
            "x86" => "x86",
            "x86_64" => "x86_64",
            "aarch64" => "arm64",
            "arm" => "arm32",
            other => other,
        }
        .to_string();
        let arch_bits = (core::mem::size_of::<usize>() * 8) as u8;
        RuntimeContext {
            os_name,
            os_version: String::new(),
            os_arch,
            arch_bits,
            features: BTreeMap::new(),
        }
    }

    /// 链式设置系统版本串（用于 os.version 正则约束）。
    pub fn with_os_version(mut self, version: impl Into<String>) -> Self {
        self.os_version = version.into();
        self
    }

    /// 链式开启/设置一个特性开关。
    pub fn with_feature(mut self, key: impl Into<String>, value: bool) -> Self {
        self.features.insert(key.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(json: &str) -> Rule {
        serde_json::from_str(json).expect("规则 JSON 应可解析")
    }

    fn win64() -> RuntimeContext {
        RuntimeContext::new(OsName::Windows, "x86_64", 64)
    }

    #[test]
    fn empty_rules_allow() {
        assert!(evaluate_rules(&[], &win64()));
    }

    #[test]
    fn allow_os_only_matches_that_os() {
        let rules = vec![rule(r#"{"action":"allow","os":{"name":"linux"}}"#)];
        assert!(!evaluate_rules(&rules, &win64()));
        assert!(evaluate_rules(
            &rules,
            &RuntimeContext::new(OsName::Linux, "x86_64", 64)
        ));
    }

    #[test]
    fn allow_then_disallow_osx_excludes_only_osx() {
        // 经典 tv.twitch 形态：全局放行，再对 osx 单独排除。
        let rules = vec![
            rule(r#"{"action":"allow"}"#),
            rule(r#"{"action":"disallow","os":{"name":"osx"}}"#),
        ];
        assert!(evaluate_rules(&rules, &win64()));
        assert!(evaluate_rules(
            &rules,
            &RuntimeContext::new(OsName::Linux, "x86_64", 64)
        ));
        assert!(!evaluate_rules(
            &rules,
            &RuntimeContext::new(OsName::Osx, "x86_64", 64)
        ));
    }

    #[test]
    fn arch_rule_only_matches_matching_arch() {
        let rules = vec![rule(r#"{"action":"allow","os":{"arch":"x86"}}"#)];
        assert!(!evaluate_rules(&rules, &win64()));
        assert!(evaluate_rules(
            &rules,
            &RuntimeContext::new(OsName::Windows, "x86", 32)
        ));
    }

    #[test]
    fn feature_rule_requires_feature_true() {
        let rules = vec![rule(r#"{"action":"allow","features":{"is_demo_user":true}}"#)];
        assert!(!evaluate_rules(&rules, &win64()));
        assert!(evaluate_rules(
            &rules,
            &win64().with_feature("is_demo_user", true)
        ));
    }

    #[test]
    fn os_version_regex_constraint() {
        let rules = vec![rule(r#"{"action":"allow","os":{"name":"windows","version":"^10\\."}}"#)];
        assert!(!evaluate_rules(&rules, &win64())); // 空版本串不匹配
        assert!(evaluate_rules(&rules, &win64().with_os_version("10.0.22631")));
        assert!(!evaluate_rules(&rules, &win64().with_os_version("6.1.7601")));
    }

    #[test]
    fn malformed_os_version_regex_does_not_match() {
        // 非法正则不应 panic，只是让该规则不命中。
        let rules = vec![rule(r#"{"action":"allow","os":{"version":"("}}"#)];
        assert!(!evaluate_rules(&rules, &win64().with_os_version("anything")));
    }
}
