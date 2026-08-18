//! 崩溃诊断门面：把日志规则命中的结果翻译成「玩家能动手的下一步」。
//!
//! 规则匹配本身在 [`aurora_launch::analyze`] 里（八类规则，各带中文 summary 与 advice）。本模块补两
//! 件门面才做得到的事：一是把日志里提取到的 mod id 拿去和该实例的卷宗 join，直接指出是磁盘上的哪个
//! jar——玩家没法拿一个 `sodium` 字符串去删文件；二是给出归档日志路径，让 UI 能提供「打开日志」。
//!
//! 文案纪律：一律表述为「日志指向 X」，绝不写「X 导致崩溃」。规则命中的是线索不是定论，混合加载器
//! 环境里一个 Mod 的报错常常是另一个 Mod 缺失造成的连锁反应，武断归因会让玩家删掉无辜的 Mod 然后
//! 继续崩。同理，[`CrashSuspect::file_name`] 对不上时只报 mod id，不猜文件。

use serde::Serialize;

use aurora_launch::crash::extract_mod_ids;
use aurora_launch::{CrashDiagnosis, analyze, has_crash_marker};

use crate::error::Result;
use crate::facade::Aurora;
use crate::ledger::Ledger;

/// 一次崩溃的完整诊断。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrashReport {
    /// 命中的规则诊断，按规则顺序排列。
    pub diagnoses: Vec<CrashDiagnosis>,
    /// 从日志里提取的 mod id 与卷宗 join 出的可疑文件名。
    pub suspects: Vec<CrashSuspect>,
    /// 归档日志文件路径（供 UI 提供「打开日志」）。
    pub log_path: Option<String>,
}

/// 一个可疑 Mod。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrashSuspect {
    /// 日志里出现的 mod id。
    pub mod_id: String,
    /// 卷宗里对得上的文件名；对不上为 `None`（只报 mod id）。
    pub file_name: Option<String>,
}

impl Aurora {
    /// 分析给定日志文本，产出诊断并与该实例的卷宗 join 出可疑文件。
    ///
    /// 卷宗文件不存在（该实例从没经本启动器装过 Mod）不是错误：诊断照出，可疑项只报 mod id。
    /// 卷宗存在但损坏则冒泡——那说明身份索引已经不可信，此时给出的文件名同样不可信。
    pub async fn diagnose_crash(&self, version_id: &str, log_text: &str) -> Result<CrashReport> {
        let ledger = self.ledger_store(version_id).load().await?;
        Ok(build_report(log_text, &ledger, None))
    }

    /// 读取该实例最近一次归档日志并诊断；无归档返回 `None`。
    pub async fn last_crash(&self, version_id: &str) -> Result<Option<CrashReport>> {
        // 日志归档跟着「本次启动实际使用的工作目录」走，隔离开与关时是两个不同的目录，因此必须与
        // 启动链路同源解析（[`Aurora::resolve_working_dir`]）。自己拼一次版本目录，就会在关掉隔离的
        // 实例上空找一场、报「没有日志」。注意这条路径与卷宗路径本就不同源：卷宗恒在
        // `versions/<id>/.aurora/`，日志在 `<工作目录>/.aurora/logs/`。
        let working_dir = self.resolve_working_dir(version_id).await?.working_dir;
        let archives = aurora_launch::logfile::list_archived_logs(&working_dir).await?;
        let Some(latest) = archives.into_iter().next() else {
            return Ok(None);
        };

        let bytes = tokio::fs::read(&latest)
            .await
            .map_err(|source| aurora_base::Error::Io {
                path: latest.clone(),
                source,
            })?;
        // 有损解码而不是拒绝：老版本 Forge 在中文 Windows 上会把 GBK 字节写进崩溃堆栈，而诊断全靠
        // 子串与正则匹配，个别乱码字符不影响命中。因为编码不干净就不出诊断，等于对最需要帮助的玩家闭嘴。
        let text = String::from_utf8_lossy(&bytes);
        let ledger = self.ledger_store(version_id).load().await?;
        Ok(Some(build_report(
            &text,
            &ledger,
            Some(latest.display().to_string()),
        )))
    }
}

/// 组装报告：规则诊断 + 与卷宗 join 后的可疑 Mod + 归档日志路径。
fn build_report(log_text: &str, ledger: &Ledger, log_path: Option<String>) -> CrashReport {
    let diagnoses = analyze(log_text);
    // 日志里没有任何崩溃迹象时不报可疑 Mod：一次正常启动的日志同样会点名 Mod（mixin 归属、依赖检查
    // 输出），照单端出来会让玩家在游戏根本没崩的情况下去删 Mod。规则命中或出现崩溃标记，才算有迹象。
    let suspects = if diagnoses.is_empty() && !has_crash_marker(log_text) {
        Vec::new()
    } else {
        extract_mod_ids(log_text)
            .into_iter()
            .map(|mod_id| {
                let file_name = ledger_file_for_mod_id(ledger, &mod_id);
                CrashSuspect { mod_id, file_name }
            })
            .collect()
    };

    CrashReport {
        diagnoses,
        suspects,
        log_path,
    }
}

/// 拿 mod id 去卷宗里找对应的磁盘文件名；对不上返回 `None`。
///
/// 卷宗记的是文件名而不是 mod id（平台元数据里根本没有 mod id 这一栏），所以只能按命名惯例匹配：
/// 模组文件名以自己的 id/slug 开头，其后接版本号分隔符（`sodium-fabric-0.5.3.jar` 之于 `sodium`）。
/// 这里不去核磁盘上文件是否还在：崩溃日志是过去某次运行的事实，那次运行时这些文件确实在，事后被删
/// 反而是玩家已经处理过的迹象，用「现在还在不在」去过滤会把线索抹掉。
fn ledger_file_for_mod_id(ledger: &Ledger, mod_id: &str) -> Option<String> {
    let mod_id = mod_id.to_ascii_lowercase();
    ledger
        .entries
        .iter()
        .find(|entry| file_name_matches_mod_id(&entry.file_name, &mod_id))
        .map(|entry| entry.file_name.clone())
}

/// 文件名是否以该 mod id 起头（忽略大小写与 `.jar` / `.disabled` 后缀）。
///
/// 只认「前缀 + 分隔符」而不认任意包含：`api` 若能匹配到 `fabric-api-*.jar`，玩家就会照着一个错误的
/// 文件名下手。宁可少给一个文件名（退化成只报 mod id），也不能指着无辜的 jar。
fn file_name_matches_mod_id(file_name: &str, mod_id: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".disabled").unwrap_or(lower.as_str());
    let stem = stem.strip_suffix(".jar").unwrap_or(stem);
    let Some(rest) = stem.strip_prefix(mod_id) else {
        return false;
    };
    rest.is_empty() || rest.starts_with(['-', '_', '+', '.', ' '])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;
    use crate::ledger::LedgerEntry;
    use aurora_launch::CrashCategory;
    use aurora_modplatform::Platform;
    use std::path::{Path, PathBuf};

    /// Fabric 1.20.1 缺前置的真实形态：Mod 列表点名 sodium，依赖检查点名 fabric-api。
    const FABRIC_MISSING_DEPENDENCY_LOG: &str = "\
[19:32:11] [main/INFO]: Loading Minecraft 1.20.1 with Fabric Loader 0.15.7
[19:32:12] [main/ERROR]: Incompatible mod set!
net.fabricmc.loader.impl.FormattedException: Incompatible mod set!
Exception in thread \"main\" java.lang.RuntimeException: Mod resolution failed
\t - Mod 'Sodium' (sodium) 0.5.3 requires version 0.90.0 or later of fabric-api, which is missing!
\tat net.fabricmc.loader.impl.launch.knot.Knot.init(Knot.java:143)
";

    /// 有崩溃标记但八条规则一条都不命中：诊断为空，可疑 Mod 仍要报出来。
    const MIXIN_INJECTION_CRASH_LOG: &str = "\
Exception in thread \"main\" java.lang.NoSuchMethodError: 'void net.minecraft.class_310.method_1507()'
\tat net.example.Hook.apply(Hook.java:31)
Caused by: org.spongepowered.asm.mixin.injection.throwables.InjectionError: Critical injection failure in handler from mod sodium
";

    /// 一次正常启动：没有崩溃迹象，但同样点名了 sodium。
    const HEALTHY_LOG: &str = "\
[19:30:02] [main/INFO]: Loading Minecraft 1.20.1 with Fabric Loader 0.15.7
[19:30:02] [main/INFO]: Loading 42 mods:
\t - Mod 'Sodium' (sodium) 0.5.3
[19:30:09] [Render thread/INFO]: OpenGL initialized: NVIDIA GeForce RTX 4070
[19:30:11] [Render thread/INFO]: Setting user: Steve
";

    fn aurora_at(root: &Path) -> Aurora {
        Aurora::for_test(
            AuroraConfig::default(),
            root.to_path_buf(),
            root.join(".minecraft"),
        )
    }

    /// 在 versions/<id>/<id>.json 落一份最小合法版本 JSON，供工作目录解析发现该版本。
    async fn put_version(mc: &Path, id: &str) {
        let dir = mc.join("versions").join(id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join(format!("{id}.json")),
            format!(r#"{{"id":"{id}","type":"release","mainClass":"m"}}"#),
        )
        .await
        .unwrap();
    }

    async fn put_ledger(aurora: &Aurora, version_id: &str, files: &[&str]) {
        let mut ledger = Ledger::default();
        for (index, file_name) in files.iter().enumerate() {
            ledger.upsert(LedgerEntry {
                file_name: (*file_name).to_owned(),
                platform: Platform::Modrinth,
                project_id: format!("proj-{index}"),
                version_id: format!("ver-{index}"),
                sha1: None,
                installed_at: 1_754_600_000 + index as u64,
                installed_as_dependency_of: None,
            });
        }
        aurora.ledger_store(version_id).save(&ledger).await.unwrap();
    }

    /// 按 logfile 契约的会话日志命名落一份归档日志，返回其路径。
    async fn put_archived_log(working_dir: &Path, started_at: u64, text: &str) -> PathBuf {
        let path = aurora_launch::logfile::session_log_path(working_dir, started_at);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, text).await.unwrap();
        path
    }

    #[tokio::test]
    async fn diagnose_crash_hits_rules_and_joins_ledger_by_file_name() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());
        put_ledger(&aurora, "1.20.1-fabric", &["sodium-fabric-0.5.3.jar"]).await;

        let report = aurora
            .diagnose_crash("1.20.1-fabric", FABRIC_MISSING_DEPENDENCY_LOG)
            .await
            .unwrap();

        // 规则侧：只命中「缺少前置」一条，且提取到缺失的工程 id。
        assert_eq!(report.diagnoses.len(), 1);
        assert_eq!(
            report.diagnoses[0].category,
            CrashCategory::MissingDependency
        );
        assert_eq!(report.diagnoses[0].detail.as_deref(), Some("fabric-api"));

        // 可疑项按日志里首次出现的顺序：sodium 在卷宗里对得上文件，fabric-api 对不上只报 id。
        assert_eq!(report.suspects.len(), 2);
        assert_eq!(report.suspects[0].mod_id, "sodium");
        assert_eq!(
            report.suspects[0].file_name.as_deref(),
            Some("sodium-fabric-0.5.3.jar")
        );
        assert_eq!(report.suspects[1].mod_id, "fabric-api");
        assert_eq!(report.suspects[1].file_name, None);

        // 传入文本诊断没有归档来源，不许编一个路径出来。
        assert_eq!(report.log_path, None);
    }

    #[tokio::test]
    async fn diagnose_crash_without_ledger_file_still_names_mod_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());

        // 卷宗文件根本不存在（从未经本启动器装过 Mod）：不报错，退化成只报 mod id。
        let report = aurora
            .diagnose_crash("1.20.1-fabric", FABRIC_MISSING_DEPENDENCY_LOG)
            .await
            .unwrap();

        let ids: Vec<&str> = report.suspects.iter().map(|s| s.mod_id.as_str()).collect();
        assert_eq!(ids, vec!["sodium", "fabric-api"]);
        assert!(report.suspects.iter().all(|s| s.file_name.is_none()));
    }

    #[tokio::test]
    async fn healthy_log_yields_no_diagnosis_and_no_suspects() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());
        put_ledger(&aurora, "1.20.1-fabric", &["sodium-fabric-0.5.3.jar"]).await;

        let report = aurora
            .diagnose_crash("1.20.1-fabric", HEALTHY_LOG)
            .await
            .unwrap();

        assert!(report.diagnoses.is_empty());
        // 这份正常日志确实点了 sodium 的名，可疑项之所以为空是「无崩溃迹象就不报」这条闸门在起作用。
        assert_eq!(extract_mod_ids(HEALTHY_LOG), vec!["sodium".to_owned()]);
        assert!(report.suspects.is_empty());
    }

    #[tokio::test]
    async fn crash_marker_without_rule_hit_still_names_suspects() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());
        put_ledger(
            &aurora,
            "1.20.1-fabric",
            &["lithium-0.11.2.jar", "sodium-fabric-0.5.3.jar"],
        )
        .await;

        let report = aurora
            .diagnose_crash("1.20.1-fabric", MIXIN_INJECTION_CRASH_LOG)
            .await
            .unwrap();

        // 八条规则一条没命中，但崩溃标记在，线索照给。
        assert!(report.diagnoses.is_empty());
        assert_eq!(report.suspects.len(), 1);
        assert_eq!(report.suspects[0].mod_id, "sodium");
        assert_eq!(
            report.suspects[0].file_name.as_deref(),
            Some("sodium-fabric-0.5.3.jar")
        );
    }

    #[test]
    fn ledger_join_requires_prefix_plus_separator() {
        let mut ledger = Ledger::default();
        for file_name in [
            "sodium-fabric-mc1.20.1-0.5.3.jar",
            "fabric-api-0.92.2+1.20.1.jar",
            "JEI_1.20.1-15.2.0.27.jar",
            "lithium.jar.disabled",
        ] {
            ledger.upsert(LedgerEntry {
                file_name: file_name.to_owned(),
                platform: Platform::Modrinth,
                project_id: "p".to_owned(),
                version_id: "v".to_owned(),
                sha1: None,
                installed_at: 0,
                installed_as_dependency_of: None,
            });
        }

        assert_eq!(
            ledger_file_for_mod_id(&ledger, "sodium").as_deref(),
            Some("sodium-fabric-mc1.20.1-0.5.3.jar")
        );
        // 带连字符的 id 与 + 号版本串照样对得上。
        assert_eq!(
            ledger_file_for_mod_id(&ledger, "fabric-api").as_deref(),
            Some("fabric-api-0.92.2+1.20.1.jar")
        );
        // 文件名大写、下划线分隔：忽略大小写后仍命中。
        assert_eq!(
            ledger_file_for_mod_id(&ledger, "jei").as_deref(),
            Some("JEI_1.20.1-15.2.0.27.jar")
        );
        // 禁用态的 jar 仍要认得出来——玩家可能已经禁用了嫌疑 Mod 再来看诊断。
        assert_eq!(
            ledger_file_for_mod_id(&ledger, "lithium").as_deref(),
            Some("lithium.jar.disabled")
        );

        // 只认前缀：api 不该指向 fabric-api-*.jar。
        assert_eq!(ledger_file_for_mod_id(&ledger, "api"), None);
        // 前缀之后必须是分隔符：sod 不该指向 sodium-*.jar。
        assert_eq!(ledger_file_for_mod_id(&ledger, "sod"), None);
        // 卷宗里没有的工程就是没有。
        assert_eq!(ledger_file_for_mod_id(&ledger, "journeymap"), None);
    }

    #[tokio::test]
    async fn last_crash_takes_latest_archive_and_reports_its_path() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());
        let mc = tmp.path().join(".minecraft");
        put_version(&mc, "1.20.1-fabric").await;

        // 一份归档都没有时不许编报告。
        assert!(aurora.last_crash("1.20.1-fabric").await.unwrap().is_none());

        let working_dir = aurora
            .resolve_working_dir("1.20.1-fabric")
            .await
            .unwrap()
            .working_dir;
        put_archived_log(&working_dir, 1_754_600_000, HEALTHY_LOG).await;
        let latest =
            put_archived_log(&working_dir, 1_754_612_345, FABRIC_MISSING_DEPENDENCY_LOG).await;
        put_ledger(&aurora, "1.20.1-fabric", &["sodium-fabric-0.5.3.jar"]).await;

        let report = aurora
            .last_crash("1.20.1-fabric")
            .await
            .unwrap()
            .expect("已有归档日志");

        // 读的是最近一份（崩溃那份），不是更早的那份正常日志。
        assert_eq!(report.diagnoses.len(), 1);
        assert_eq!(
            report.diagnoses[0].category,
            CrashCategory::MissingDependency
        );
        assert_eq!(
            report.suspects[0].file_name.as_deref(),
            Some("sodium-fabric-0.5.3.jar")
        );
        assert_eq!(
            report.log_path.as_deref(),
            Some(latest.display().to_string().as_str())
        );
    }

    #[tokio::test]
    async fn last_crash_on_uninstalled_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());
        tokio::fs::create_dir_all(tmp.path().join(".minecraft").join("versions"))
            .await
            .unwrap();

        // 版本没装就没有工作目录可言，冒泡而不是伪装成「没有日志」。
        let err = aurora.last_crash("ghost").await.unwrap_err();
        match err {
            crate::error::CoreError::VersionNotInstalled { id } => assert_eq!(id, "ghost"),
            other => panic!("期望版本未安装错误，得到 {other:?}"),
        }
    }

    #[test]
    fn report_serializes_with_ipc_field_names() {
        let report = CrashReport {
            diagnoses: vec![CrashDiagnosis {
                category: CrashCategory::MissingDependency,
                summary: "日志指向缺失的前置 Mod".to_owned(),
                advice: "安装日志中提到的前置 Mod 后重试".to_owned(),
                detail: Some("fabric-api".to_owned()),
                matched: "Missing mod: fabric-api".to_owned(),
            }],
            suspects: vec![
                CrashSuspect {
                    mod_id: "sodium".to_owned(),
                    file_name: Some("sodium-0.5.3.jar".to_owned()),
                },
                CrashSuspect {
                    mod_id: "fabric-api".to_owned(),
                    file_name: None,
                },
            ],
            log_path: Some("D:\\mc\\versions\\1.20.1\\.aurora\\logs\\1754612345.log".to_owned()),
        };

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["diagnoses"][0]["summary"], "日志指向缺失的前置 Mod");
        assert_eq!(json["diagnoses"][0]["detail"], "fabric-api");
        // 卷宗 join 命中的给文件名，没命中的只给 mod id——不猜。
        assert_eq!(json["suspects"][0]["mod_id"], "sodium");
        assert_eq!(json["suspects"][0]["file_name"], "sodium-0.5.3.jar");
        assert_eq!(json["suspects"][1]["mod_id"], "fabric-api");
        assert_eq!(json["suspects"][1]["file_name"], serde_json::Value::Null);
        assert_eq!(
            json["log_path"],
            "D:\\mc\\versions\\1.20.1\\.aurora\\logs\\1754612345.log"
        );
    }

    #[test]
    fn report_without_archive_keeps_arrays_present() {
        // 无归档日志时 log_path 为 null，但两个数组字段必须仍然存在，UI 直接读长度。
        let json = serde_json::to_value(CrashReport {
            diagnoses: Vec::new(),
            suspects: Vec::new(),
            log_path: None,
        })
        .unwrap();
        assert_eq!(json["diagnoses"], serde_json::json!([]));
        assert_eq!(json["suspects"], serde_json::json!([]));
        assert_eq!(json["log_path"], serde_json::Value::Null);
    }
}
