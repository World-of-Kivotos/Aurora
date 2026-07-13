//! 离线启动编排：账户 -> 版本合并 -> Java 解析 -> 启动前检查 -> 命令拼装 -> 进程 spawn。
//!
//! 门面把 aurora-instance（版本发现/隔离）、aurora-version（继承合并）、aurora-java（探测/自动下载）、
//! aurora-launch（命令拼装/进程/启动前检查）串成一次可执行的离线启动。启动前只做轻量存在性检查
//! （启动前检查报告），深度文件补全（`ensure_complete`）由安装流程负责，不在每次启动时重跑逐文件哈希。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aurora_instance::{IsolationOverride, discover_versions};
use aurora_java::{DetectSource, JavaInstallation, JavaRuntimeInstaller, detect_all, select_for_major};
use aurora_launch::{
    AuthValues, CheckStatus, CommandBuilder, GamePaths, GameSession, LaunchCommand, LogLine,
    MemoryConfig, PreLaunchInput, precheck, resolve_game_directory, spawn,
};
use aurora_version::{RuntimeContext, VersionJson, resolve};
use tokio::sync::mpsc;

use crate::error::{CoreError, Result};
use crate::event::{CoreEvent, EventSink, emit};
use crate::facade::Aurora;

/// 离线启动的可覆盖选项（缺省取全局配置）。
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// 最大堆（MB）；缺省取配置的内存设置。
    pub max_memory_mb: Option<u32>,
    /// 最小堆（MB）；缺省取配置的内存设置。
    pub min_memory_mb: Option<u32>,
    /// 是否全屏启动。
    pub fullscreen: bool,
}

impl Aurora {
    /// 以离线账户启动指定的已安装版本，返回运行中的游戏会话。
    ///
    /// `log_tx` 提供时逐行转发游戏进程输出（供 CLI/前端实时展示）。启动前会做轻量检查（主类、客户端
    /// jar 存在、Java 匹配、路径字符）；任一阻断项都会中止并冒泡。
    pub async fn launch_offline(
        &self,
        version_id: &str,
        player_name: &str,
        options: &LaunchOptions,
        log_tx: Option<mpsc::Sender<LogLine>>,
        events: Option<&EventSink>,
    ) -> Result<GameSession> {
        let account = self.create_offline_account(player_name, events)?;

        // 版本发现 + 继承合并。
        let scan = discover_versions(self.game_dir()).await?;
        let mut provider: HashMap<String, VersionJson> = HashMap::new();
        for version in &scan.versions {
            provider.insert(version.id.clone(), version.json.clone());
        }
        let target = scan
            .versions
            .iter()
            .find(|v| v.id == version_id)
            .ok_or_else(|| CoreError::VersionNotInstalled {
                id: version_id.to_owned(),
            })?;
        let merged = resolve(&target.json, &provider)?;
        // 客户端 jar 与 natives 落在继承链根（原版）版本目录；离线原版即自身。
        let base_id = resolve_base_id(&provider, version_id);

        // Java 解析（本地匹配优先，缺失且允许时自动下载），产出探测列表供启动前检查复用。
        let required_major = merged
            .java_version
            .as_ref()
            .map(|j| j.major_version)
            .unwrap_or(8);
        let (installations, java_path) = self.prepare_java(required_major, events).await?;

        // 版本隔离判定，产出工作目录。
        let resolved = resolve_game_directory(
            self.game_dir(),
            version_id,
            self.config().isolation_policy,
            IsolationOverride::FollowGlobal,
            target.has_mod_loader(),
            target.is_release(),
        )
        .await?;
        let working_dir = resolved.working_dir.clone();
        tokio::fs::create_dir_all(&working_dir)
            .await
            .map_err(|source| aurora_base::Error::Io {
                path: working_dir.clone(),
                source,
            })?;

        let paths = GamePaths::standard(self.game_dir(), &base_id);

        // 启动前检查。
        let report = precheck::run(&PreLaunchInput {
            game_dir: &working_dir,
            version: &merged,
            java_installations: &installations,
            account: &account,
            now_unix: now_unix(),
            client_jar: &paths.client_jar,
        });
        for item in &report.items {
            if item.status == CheckStatus::Warn {
                emit(events, CoreEvent::warning(format!("{}：{}", item.name, item.message)));
            }
        }
        if report.is_blocking() {
            let failures = report
                .items
                .iter()
                .filter(|i| i.status == CheckStatus::Fail)
                .map(|i| format!("{}：{}", i.name, i.message))
                .collect::<Vec<_>>()
                .join("；");
            return Err(CoreError::PrecheckFailed(failures));
        }

        let memory = self.memory_config(options);
        let command = assemble_command(
            &merged,
            &java_path,
            paths,
            &working_dir,
            &account.name,
            &account.uuid,
            version_id,
            self.runtime().clone(),
            memory,
            options.fullscreen,
        )?;
        emit(
            events,
            CoreEvent::stage(format!(
                "启动命令已就绪：Java {} 主类 {}",
                java_path.display(),
                merged.main_class.as_deref().unwrap_or("<未知>")
            )),
        );

        let session = spawn(&command, log_tx)?;
        emit(
            events,
            CoreEvent::stage(format!(
                "游戏进程已启动（工作目录 {}{}）",
                working_dir.display(),
                if resolved.isolated { "，已隔离" } else { "" }
            )),
        );
        Ok(session)
    }

    /// 解析可用 Java：本地探测并按主版本匹配，缺失且允许时自动下载 Mojang 运行时。
    ///
    /// 返回 `(探测/托管的 Java 列表, 选中的 java 可执行文件路径)`；列表含选中的 Java，供启动前检查复用。
    pub(crate) async fn prepare_java(
        &self,
        required_major: u32,
        events: Option<&EventSink>,
    ) -> Result<(Vec<JavaInstallation>, PathBuf)> {
        let mut installations = tokio::task::spawn_blocking(detect_all).await?;
        if let Some(java) = select_for_major(&installations, required_major) {
            let path = java.path.clone();
            emit(
                events,
                CoreEvent::stage(format!("使用本地 Java {required_major}：{}", path.display())),
            );
            return Ok((installations, path));
        }

        if !self.config().auto_download_java {
            return Err(CoreError::NoJava {
                major: required_major,
            });
        }

        emit(
            events,
            CoreEvent::stage(format!("本地未找到 Java {required_major}，开始下载 Mojang 运行时")),
        );
        let installer = JavaRuntimeInstaller::new(self.http())
            .with_manifest_url(self.java_runtime_url())
            .with_source(self.config().download_source.primary_mirror());
        let install_dir = self
            .data_dir()
            .join("runtime")
            .join(required_major.to_string());
        let runtime = installer.install(required_major, &install_dir).await?;
        let exe = runtime.java_executable.clone();
        // 探测刚下载的运行时，纳入列表让启动前检查的 Java 项通过。
        let probed =
            tokio::task::spawn_blocking(move || aurora_java::probe(&exe, DetectSource::Managed))
                .await??;
        let path = probed.path.clone();
        installations.push(probed);
        emit(
            events,
            CoreEvent::stage(format!("Java {required_major} 运行时安装完成：{}", path.display())),
        );
        Ok((installations, path))
    }

    /// 由启动选项与全局配置算出内存配置。
    fn memory_config(&self, options: &LaunchOptions) -> MemoryConfig {
        let max = options.max_memory_mb.unwrap_or(self.config().memory.max_mb);
        let mut config = MemoryConfig::fixed(max);
        if let Some(min) = options.min_memory_mb.or(self.config().memory.min_mb) {
            config = config.with_min(min);
        }
        config
    }
}

/// 沿 `inheritsFrom` 链找到继承根版本 id（持有客户端 jar / natives 的版本）。
///
/// 原版无 `inheritsFrom`，返回自身；加载器版本返回其原版 id。带环保护（异常数据不至死循环）。
fn resolve_base_id(provider: &HashMap<String, VersionJson>, root_id: &str) -> String {
    let mut current = root_id.to_owned();
    let mut seen = std::collections::HashSet::new();
    while seen.insert(current.clone()) {
        match provider.get(&current).and_then(|v| v.inherits_from.clone()) {
            Some(parent) => current = parent,
            None => break,
        }
    }
    current
}

/// 用合并后的版本 JSON、路径与离线鉴权值拼出可执行的启动命令。
#[allow(clippy::too_many_arguments)]
fn assemble_command(
    merged: &VersionJson,
    java: &Path,
    paths: GamePaths,
    working_dir: &Path,
    account_name: &str,
    account_uuid: &str,
    version_name: &str,
    runtime: RuntimeContext,
    memory: MemoryConfig,
    fullscreen: bool,
) -> Result<LaunchCommand> {
    let auth = AuthValues::offline(account_name, account_uuid);
    let command = CommandBuilder::new(merged, runtime, java, paths, working_dir, auth)
        .with_memory(memory)
        .with_version_name(version_name)
        .fullscreen(fullscreen)
        .build()?;
    Ok(command)
}

/// 当前 Unix 秒（时钟异常时退化为 0，令令牌视为过期，属安全兜底）。
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(json: &str) -> VersionJson {
        VersionJson::from_json_str(json).unwrap()
    }

    #[test]
    fn base_id_walks_inheritance_chain() {
        let mut provider = HashMap::new();
        provider.insert(
            "1.21".to_owned(),
            version(r#"{"id":"1.21","type":"release","mainClass":"m"}"#),
        );
        provider.insert(
            "fabric-loader-0.15.11-1.21".to_owned(),
            version(
                r#"{"id":"fabric-loader-0.15.11-1.21","inheritsFrom":"1.21","mainClass":"knot"}"#,
            ),
        );

        // 加载器版本 -> 原版根。
        assert_eq!(
            resolve_base_id(&provider, "fabric-loader-0.15.11-1.21"),
            "1.21"
        );
        // 原版 -> 自身。
        assert_eq!(resolve_base_id(&provider, "1.21"), "1.21");
        // 未知 id -> 原样返回。
        assert_eq!(resolve_base_id(&provider, "ghost"), "ghost");
    }

    #[test]
    fn base_id_survives_cyclic_data() {
        let mut provider = HashMap::new();
        provider.insert(
            "a".to_owned(),
            version(r#"{"id":"a","inheritsFrom":"b","mainClass":"m"}"#),
        );
        provider.insert(
            "b".to_owned(),
            version(r#"{"id":"b","inheritsFrom":"a","mainClass":"m"}"#),
        );
        // 环不应死循环；返回环上某个 id（不 panic 即可）。
        let base = resolve_base_id(&provider, "a");
        assert!(base == "a" || base == "b");
    }

    #[test]
    fn assemble_offline_command_injects_auth_memory_and_main_class() {
        let merged = version(
            r#"{"id":"1.21","type":"release","mainClass":"net.minecraft.client.main.Main",
                "minecraftArguments":"--username ${auth_player_name} --uuid ${auth_uuid} --accessToken ${auth_access_token}"}"#,
        );
        let runtime = RuntimeContext::new(aurora_version::OsName::Windows, "x86_64", 64);
        let paths = GamePaths::standard(PathBuf::from("D:/mc/.minecraft"), "1.21");
        let command = assemble_command(
            &merged,
            Path::new("C:/java/bin/java.exe"),
            paths,
            Path::new("D:/mc/.minecraft"),
            "Steve",
            "5627dd98e6be3c21b8a8e92344183641",
            "1.21",
            runtime,
            MemoryConfig::fixed(3072).with_min(512),
            false,
        )
        .unwrap();

        assert_eq!(command.program, PathBuf::from("C:/java/bin/java.exe"));
        assert_eq!(command.working_dir, PathBuf::from("D:/mc/.minecraft"));
        // 内存参数（-Xms 在前、-Xmx 在后）。
        assert!(command.args.iter().any(|a| a == "-Xms512m"));
        assert!(command.args.iter().any(|a| a == "-Xmx3072m"));
        // 主类。
        assert!(command.args.iter().any(|a| a == "net.minecraft.client.main.Main"));
        // 离线鉴权值经占位符替换进入游戏参数。
        let joined = command.args.join(" ");
        assert!(joined.contains("--username Steve"));
        assert!(joined.contains("--uuid 5627dd98e6be3c21b8a8e92344183641"));
        // 离线访问令牌占位常量 "0"。
        assert!(joined.contains("--accessToken 0"));
    }
}
