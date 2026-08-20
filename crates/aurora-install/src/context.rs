//! 安装流程的共享上下文与批量下载收口。
//!
//! 各安装器（原版 / Fabric / Quilt / Forge）都需要同一组协作对象：注入的 HTTP 客户端（读元数据）、
//! aurora-download 并发池（下文件）、目录布局、运行环境与重试策略。[`InstallContext`] 把它们打成一束
//! 借用引用（整体 `Copy`，按值传递零成本），避免每个方法拖一长串参数。

use aurora_base::retry::RetryPolicy;
use aurora_download::{DownloadPool, DownloadProgress, DownloadTask};
use aurora_version::RuntimeContext;
use tokio::sync::watch;

use crate::error::{Error, Result};
use crate::layout::GameLayout;

/// 安装流程共享的协作对象（全为借用，`Copy`）。
#[derive(Clone, Copy)]
pub struct InstallContext<'a> {
    /// 读元数据用的 HTTP 客户端（由 aurora-base 工厂构建）。
    pub client: &'a reqwest::Client,
    /// 下文件用的并发池（内含单文件引擎与镜像源调度）。
    pub pool: &'a DownloadPool,
    /// 目标游戏根目录布局。
    pub layout: &'a GameLayout,
    /// 运行环境（决定 library rules 与 natives classifier）。
    pub runtime: &'a RuntimeContext,
    /// 元数据抓取的退避重试策略（文件下载自有 aurora-download 的策略）。
    pub policy: &'a RetryPolicy,
    /// 批量下载的进度观察者，缺省不挂（[`InstallContext::with_progress`] 挂上）。
    ///
    /// 安装一个版本的耗时几乎全在下文件上（光资源对象就三千多个），而安装器本身只在首尾发两句
    /// 阶段文案。上层要画进度条，就得拿到这些批次的文件计数——挂上这个观察者是唯一的入口。
    progress: Option<&'a watch::Sender<DownloadProgress>>,
}

impl<'a> InstallContext<'a> {
    /// 组装上下文。
    pub fn new(
        client: &'a reqwest::Client,
        pool: &'a DownloadPool,
        layout: &'a GameLayout,
        runtime: &'a RuntimeContext,
        policy: &'a RetryPolicy,
    ) -> Self {
        Self {
            client,
            pool,
            layout,
            runtime,
            policy,
            progress: None,
        }
    }

    /// 挂上批量下载的进度观察者：此后本上下文跑的每一批下载都会向它推送快照。
    ///
    /// 注意批次之间计数会归零重来（原版是「版本 JSON / 本体与库 / 资源对象」三批依次下），
    /// 每批各自从 0/total 起算。这是真实的批次边界，不在这里做平滑或累加：
    /// 观察者拿到的必须是「这一批下到哪了」的原始数，怎么折算成进度条是上层的事。
    pub fn with_progress(mut self, progress: &'a watch::Sender<DownloadProgress>) -> Self {
        self.progress = Some(progress);
        self
    }

    /// 跑一批下载并把「有文件最终失败」收敛为 [`Error::BatchIncomplete`]。
    ///
    /// aurora-download 的批量池对单文件失败返回 `Ok(report)` 而非 `Err`，故这里必须显式检查
    /// `is_success()`，绝不放过部分失败（缺文件会在启动时才炸，代价更高）。
    pub(crate) async fn run_batch(
        &self,
        tasks: Vec<DownloadTask>,
        stage: &'static str,
    ) -> Result<usize> {
        if tasks.is_empty() {
            return Ok(0);
        }
        // 观察者按批次克隆下发：池在批次收尾时会 abort 采样器并推最后一帧，
        // 故多批共用一个发送端不会留下还在推陈旧快照的后台任务。
        let report = self.pool.download_all(tasks, self.progress.cloned()).await?;
        if report.is_success() {
            Ok(report.succeeded)
        } else {
            Err(Error::BatchIncomplete {
                stage,
                total: report.total,
                failed: report.failures.len(),
            })
        }
    }
}
