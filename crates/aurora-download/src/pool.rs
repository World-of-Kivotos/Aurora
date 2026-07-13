//! 批量并发下载池。
//!
//! [`DownloadPool`] 用一个信号量控住「同时在下的文件数」，把上千个小文件（如 assets）以合并
//! 并发池的形式跑完，并把总进度经 [`watch`] channel 推给前端。发起前按目标路径去重，避免重复入队
//! 同名任务。单个文件失败（重试并换源后仍失败）不拖垮整批：失败被收集进 [`BatchReport`]，由调用方
//! 决定后续处置。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;

use crate::engine::Downloader;
use crate::error::{Error, Result};
use crate::progress::{DownloadProgress, ProgressReporter};
use crate::task::DownloadTask;

/// 单个文件的失败记录。
#[derive(Debug)]
pub struct TaskFailure {
    /// 失败的任务。
    pub task: DownloadTask,
    /// 重试并换源后仍未克服的最终错误。
    pub error: Error,
}

/// 一批下载的结果汇总。
#[derive(Debug)]
pub struct BatchReport {
    /// 去重后的任务总数。
    pub total: usize,
    /// 成功数。
    pub succeeded: usize,
    /// 失败明细。
    pub failures: Vec<TaskFailure>,
}

impl BatchReport {
    /// 是否整批无失败。调用方应据此判定这批下载可否进入下一步。
    pub fn is_success(&self) -> bool {
        self.failures.is_empty()
    }
}

/// 批量并发下载池。
pub struct DownloadPool {
    downloader: Downloader,
    file_concurrency: usize,
    sample_interval: Duration,
}

impl DownloadPool {
    /// 用给定引擎与并发上限构造。采样间隔默认 100ms（对齐进度刷新节奏）。
    pub fn new(downloader: Downloader, file_concurrency: usize) -> Self {
        Self {
            downloader,
            file_concurrency: file_concurrency.max(1),
            sample_interval: Duration::from_millis(100),
        }
    }

    /// 覆盖进度采样间隔。
    pub fn with_sample_interval(mut self, interval: Duration) -> Self {
        self.sample_interval = interval;
        self
    }

    /// 批量下载。`sender` 提供则全程推送进度快照（含起始与最终快照）。
    ///
    /// 返回汇总报告：即便有文件失败也返回 `Ok(report)`，失败明细在 `report.failures`。仅当某个
    /// 下载任务本身 panic（而非下载失败）才返回 `Err`。调用方务必检查 [`BatchReport::is_success`]。
    pub async fn download_all(
        &self,
        tasks: Vec<DownloadTask>,
        sender: Option<watch::Sender<DownloadProgress>>,
    ) -> Result<BatchReport> {
        let tasks = dedup_by_dest(tasks);
        let total = tasks.len();
        let reporter = ProgressReporter::new(total as u64, sender);
        reporter.publish(); // 先把 total 发出去，前端立刻能画出「0/total」。

        let sampler = if reporter.has_sender() {
            Some(reporter.spawn_sampler(self.sample_interval))
        } else {
            None
        };

        let semaphore = Arc::new(Semaphore::new(self.file_concurrency));
        let mut set: JoinSet<(usize, Result<()>)> = JoinSet::new();
        for (index, task) in tasks.iter().cloned().enumerate() {
            // 先取信号量再 spawn：并发满时在此自然阻塞，起到「文件级并发上限」的节流作用。
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("文件信号量未关闭");
            let downloader = self.downloader.clone();
            let reporter = reporter.clone();
            set.spawn(async move {
                let _permit = permit;
                let result = downloader.run(&task, Some(&reporter)).await;
                reporter.finish_file();
                (index, result)
            });
        }

        let mut succeeded = 0usize;
        let mut failures = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((_, Ok(()))) => succeeded += 1,
                Ok((index, Err(err))) => failures.push(TaskFailure {
                    task: tasks[index].clone(),
                    error: err,
                }),
                Err(join) => return Err(Error::ChunkTaskJoin(join)),
            }
        }

        if let Some(handle) = sampler {
            handle.abort();
        }
        reporter.finalize();

        Ok(BatchReport {
            total,
            succeeded,
            failures,
        })
    }
}

/// 按目标路径去重，保留首次出现，丢弃后续同目标任务（下载重复任务检测）。
fn dedup_by_dest(tasks: Vec<DownloadTask>) -> Vec<DownloadTask> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        if seen.insert(task.dest.clone()) {
            out.push(task);
        } else {
            tracing::debug!(dest = %task.dest.display(), "已存在同目标任务，跳过重复入队");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn dedup_keeps_first_by_dest() {
        let tasks = vec![
            DownloadTask::new("https://a/1", "out/x.jar"),
            DownloadTask::new("https://b/2", "out/x.jar"), // 同目标，丢弃
            DownloadTask::new("https://c/3", "out/y.jar"),
        ];
        let deduped = dedup_by_dest(tasks);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].url, "https://a/1");
        assert_eq!(deduped[0].dest, PathBuf::from("out/x.jar"));
        assert_eq!(deduped[1].dest, PathBuf::from("out/y.jar"));
    }

    #[test]
    fn empty_report_is_success() {
        let report = BatchReport {
            total: 0,
            succeeded: 0,
            failures: Vec::new(),
        };
        assert!(report.is_success());
    }
}
