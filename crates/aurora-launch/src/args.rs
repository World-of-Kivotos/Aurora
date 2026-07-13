//! JVM / 游戏参数拼装的构件：安全与编码防御参数、GC 策略、旧版固定 JVM 基座、新式条件参数展开、
//! 旧式参数切分，以及带引号感知的自定义参数分割与去重/覆盖合并。
//!
//! 这些都是纯函数，输出仍含 `${}` 占位符（由 [`crate::placeholder`] 后置替换），便于对「拼出的模板序列」
//! 做表驱动断言。

use std::collections::HashSet;

use aurora_version::{Argument, RuntimeContext, evaluate_rules};
use serde::{Deserialize, Serialize};

/// 强制安全参数：缓解 Log4Shell（CVE-2021-44228）。必须默认注入，与用户设置无关。
pub fn security_args() -> Vec<String> {
    vec!["-Dlog4j2.formatMsgNoLookups=true".to_owned()]
}

/// 编码防御参数：统一 UTF-8，解决中文环境下命令行/日志/文件名乱码。
///
/// `stdout.encoding` / `stderr.encoding` 是 Java 18+ 才识别的键，老版本忽略无害；`file.encoding` 全版本有效。
pub fn encoding_args() -> Vec<String> {
    vec![
        "-Dfile.encoding=UTF-8".to_owned(),
        "-Dstdout.encoding=UTF-8".to_owned(),
        "-Dstderr.encoding=UTF-8".to_owned(),
    ]
}

/// GC 策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GcPolicy {
    /// Mojang 官启同款 G1GC 基础组合。
    G1,
    /// 「优化的 G1GC」：在 G1 基础上关掉自适应大小策略、补齐若干调优项，追求更稳的帧时间。
    OptimizedG1,
    /// 分代 ZGC（Java 21+）。低于 21 时自动回退到 [`GcPolicy::OptimizedG1`]。
    GenerationalZgc,
}

/// 按 GC 策略与 Java 主版本产出 GC 相关参数。
pub fn gc_args(policy: GcPolicy, java_major: u32) -> Vec<String> {
    match policy {
        GcPolicy::G1 => vec![
            "-XX:+UseG1GC".to_owned(),
            "-XX:G1NewSizePercent=20".to_owned(),
            "-XX:G1ReservePercent=20".to_owned(),
            "-XX:MaxGCPauseMillis=50".to_owned(),
            "-XX:G1HeapRegionSize=16M".to_owned(),
        ],
        GcPolicy::OptimizedG1 => vec![
            "-XX:+UseG1GC".to_owned(),
            "-XX:-UseAdaptiveSizePolicy".to_owned(),
            "-XX:-OmitStackTraceInFastThrow".to_owned(),
            "-XX:G1NewSizePercent=20".to_owned(),
            "-XX:G1ReservePercent=20".to_owned(),
            "-XX:MaxGCPauseMillis=50".to_owned(),
            "-XX:G1HeapRegionSize=32M".to_owned(),
        ],
        GcPolicy::GenerationalZgc => {
            if java_major >= 21 {
                vec!["-XX:+UseZGC".to_owned(), "-XX:+ZGenerational".to_owned()]
            } else {
                // 该 Java 不支持分代 ZGC，回退到优化 G1，避免启动即报无法识别的 GC 选项。
                gc_args(GcPolicy::OptimizedG1, java_major)
            }
        }
    }
}

/// 旧版（无 `arguments.jvm`）的固定 JVM 基础参数集，模板形式。
///
/// 1.12- 的版本 JSON 不带结构化 jvm 参数，启动器需自行补上 natives 路径、启动器品牌标识与 classpath。
pub fn legacy_jvm_base_args() -> Vec<String> {
    vec![
        "-Djava.library.path=${natives_directory}".to_owned(),
        "-Dminecraft.launcher.brand=${launcher_name}".to_owned(),
        "-Dminecraft.launcher.version=${launcher_version}".to_owned(),
        "-cp".to_owned(),
        "${classpath}".to_owned(),
    ]
}

/// 展开新式 `arguments`（game 或 jvm）数组：纯字符串直接收，条件块按 rules 命中才收其 value。
///
/// 返回仍是含 `${}` 的模板序列。
pub fn expand_arguments(arguments: &[Argument], ctx: &RuntimeContext) -> Vec<String> {
    let mut out = Vec::new();
    for argument in arguments {
        match argument {
            Argument::Plain(value) => out.push(value.clone()),
            Argument::Conditional { rules, value } => {
                if evaluate_rules(rules, ctx) {
                    out.extend(value.as_slice().iter().cloned());
                }
            }
        }
    }
    out
}

/// 旧式 `minecraftArguments`：按空白切成模板 token。占位符替换在切分之后进行，因此某个值即便含空格
/// （替换后才出现）也仍是单个参数。
pub fn split_legacy_arguments(arguments: &str) -> Vec<String> {
    arguments.split_whitespace().map(str::to_owned).collect()
}

/// 带引号 / 转义感知的参数分割，供解析用户自定义参数串。
///
/// 规则：双引号内空白不切分；引号内 `\"` `\\` 为转义；显式空引号 `""` 产出一个空参数。
pub fn split_args(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if in_quotes => {
                match chars.peek() {
                    Some(&next @ ('"' | '\\')) => {
                        current.push(next);
                        chars.next();
                    }
                    _ => current.push('\\'),
                }
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    out.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(current);
    }
    out
}

/// JVM 参数去重：**完全相同**的字符串才去重（保序保留首个）。
///
/// 不做键级去重——`-Xmx2g` 与 `-Xmx4g` 是两个不同字符串，都保留（交由 JVM「后者生效」），也因此不会把
/// `-XX:+UseG1GC` 之类的开关误并。
pub fn dedup_jvm_args(args: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        if seen.insert(arg.clone()) {
            out.push(arg);
        }
    }
    out
}

/// 游戏参数合并：`extra` 里的 `--key value` 覆盖 `base` 中同名键的值；`--tweakClass` 是例外，累加不覆盖
/// （多个 tweaker 需并存）。游离项（无值的标志、或不成对的 token）直接追加。
///
/// 「键」的判定：以 `--` 开头且其后首字符是字母。据此 `-100`、`-Xmx2g` 这类不会被误判为键，避免负数值
/// 被当成新键（PCL 明确点名的坑）。
pub fn merge_game_args(base: Vec<String>, extra: Vec<String>) -> Vec<String> {
    let mut result = base;
    let mut i = 0;
    while i < extra.len() {
        let token = &extra[i];
        let paired_value = extra.get(i + 1).filter(|next| !is_key(next));
        match paired_value {
            Some(value) if is_key(token) => {
                if token == "--tweakClass" {
                    result.push(token.clone());
                    result.push(value.clone());
                } else if let Some(pos) = find_key(&result, token) {
                    result[pos + 1] = value.clone();
                } else {
                    result.push(token.clone());
                    result.push(value.clone());
                }
                i += 2;
            }
            _ => {
                result.push(token.clone());
                i += 1;
            }
        }
    }
    result
}

/// 是否是 `--key` 形式的键（`--` 后首字符为字母）。
fn is_key(token: &str) -> bool {
    token
        .strip_prefix("--")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|c| c.is_alphabetic())
}

/// 在已合并序列里找到某键的位置（其后必须紧跟一个非键值）。
fn find_key(args: &[String], key: &str) -> Option<usize> {
    args.iter().enumerate().find_map(|(idx, token)| {
        if token == key && args.get(idx + 1).is_some_and(|next| !is_key(next)) {
            Some(idx)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_version::{ArgumentValue, OsName, OsRule, Rule, RuleAction};

    fn ctx(os: OsName) -> RuntimeContext {
        RuntimeContext::new(os, "x86_64", 64)
    }

    /// 构造「仅在某系统生效」的 os 规则。
    fn os_allow(name: &str) -> Rule {
        Rule {
            action: RuleAction::Allow,
            os: Some(OsRule {
                name: Some(name.to_owned()),
                version: None,
                arch: None,
            }),
            features: None,
        }
    }

    #[test]
    fn security_and_encoding_are_fixed() {
        assert_eq!(security_args(), vec!["-Dlog4j2.formatMsgNoLookups=true"]);
        assert!(encoding_args().contains(&"-Dfile.encoding=UTF-8".to_string()));
    }

    #[test]
    fn gc_zgc_falls_back_below_java21() {
        assert_eq!(
            gc_args(GcPolicy::GenerationalZgc, 21),
            vec!["-XX:+UseZGC", "-XX:+ZGenerational"]
        );
        // Java 17 上分代 ZGC 不可用，回退到优化 G1。
        assert_eq!(
            gc_args(GcPolicy::GenerationalZgc, 17),
            gc_args(GcPolicy::OptimizedG1, 17)
        );
    }

    #[test]
    fn expand_arguments_respects_rules() {
        let game = vec![
            Argument::Plain("--always".to_owned()),
            Argument::Conditional {
                rules: vec![os_allow("windows")],
                value: ArgumentValue::Single("--win-only".to_owned()),
            },
            Argument::Conditional {
                rules: vec![os_allow("osx")],
                value: ArgumentValue::Many(vec!["--mac".to_owned(), "x".to_owned()]),
            },
        ];
        let expanded = expand_arguments(&game, &ctx(OsName::Windows));
        // windows 命中，osx 不命中。
        assert_eq!(expanded, vec!["--always", "--win-only"]);
    }

    #[test]
    fn split_legacy_collapses_whitespace() {
        assert_eq!(
            split_legacy_arguments("--username ${auth_player_name}   --uuid ${auth_uuid}"),
            vec!["--username", "${auth_player_name}", "--uuid", "${auth_uuid}"]
        );
    }

    #[test]
    fn split_args_is_quote_and_escape_aware() {
        assert_eq!(
            split_args(r#"-Xmx2g -Dfoo="a b" bar"#),
            vec!["-Xmx2g", "-Dfoo=a b", "bar"]
        );
        // 引号内的转义引号与反斜杠。
        assert_eq!(split_args(r#""a\"b" c"#), vec![r#"a"b"#, "c"]);
        // 显式空引号 -> 一个空参数。
        assert_eq!(split_args(r#"a "" b"#), vec!["a", "", "b"]);
    }

    #[test]
    fn dedup_jvm_keeps_first_and_drops_exact_dupes() {
        let args = vec![
            "-Xmx2g".to_string(),
            "-XX:+UseG1GC".to_string(),
            "-Xmx2g".to_string(),
            "-Xmx4g".to_string(),
        ];
        assert_eq!(
            dedup_jvm_args(args),
            vec!["-Xmx2g", "-XX:+UseG1GC", "-Xmx4g"]
        );
    }

    #[test]
    fn merge_game_args_overrides_except_tweakclass() {
        let base = vec![
            "--username".to_string(),
            "Steve".to_string(),
            "--tweakClass".to_string(),
            "optifine.OptiFineTweaker".to_string(),
        ];
        let extra = vec![
            "--username".to_string(),
            "Alex".to_string(),
            "--tweakClass".to_string(),
            "net.fabricmc.FabricTweaker".to_string(),
            "--width".to_string(),
            "800".to_string(),
        ];
        let merged = merge_game_args(base, extra);
        // username 被覆盖为 Alex；两个 tweakClass 并存；width 追加。
        assert_eq!(
            merged,
            vec![
                "--username",
                "Alex",
                "--tweakClass",
                "optifine.OptiFineTweaker",
                "--tweakClass",
                "net.fabricmc.FabricTweaker",
                "--width",
                "800",
            ]
        );
    }

    #[test]
    fn merge_game_args_does_not_treat_negative_value_as_key() {
        let base = vec!["--height".to_string(), "480".to_string()];
        // 负数值不应被当成新键；--height 的值被覆盖为 -100。
        let extra = vec!["--height".to_string(), "-100".to_string()];
        assert_eq!(merge_game_args(base, extra), vec!["--height", "-100"]);
    }
}
