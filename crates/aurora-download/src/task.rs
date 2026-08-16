//! 下载任务描述。
//!
//! 一个 [`DownloadTask`] 是下载引擎的最小工作单元：基准 URL、可选的任务级候选源、落盘目标，
//! 以及可选的完整性契约（sha1 与大小）。sha1 一旦提供，引擎会在合并后强制校验、不符即重下；
//! 大小一旦提供，会用于判定是否值得多线程分块以及最终的截断检查。

use std::path::PathBuf;

use aurora_base::mirror::MirrorSource;

/// 单个文件的下载任务。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadTask {
    /// 基准 URL。没有任务级候选源时，按 [`crate::source::SourcePlan`] 的全局源顺序展开。
    pub url: String,
    /// 该任务独有的候选源。非空时覆盖全局源顺序，适用于清单为每个文件提供不同 URL 的场景。
    pub task_sources: Vec<MirrorSource>,
    /// 落盘目标的完整路径，父目录会被自动创建。
    pub dest: PathBuf,
    /// 期望的 sha1（小写或大写均可）。提供则合并后校验，不符触发重下/换源。
    pub sha1: Option<String>,
    /// 期望的文件大小（字节）。提供则用于分块决策与最终截断检查。
    pub size: Option<u64>,
}

impl DownloadTask {
    /// 构造一个只含 URL 与目标路径、无完整性契约的任务。
    pub fn new(url: impl Into<String>, dest: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            task_sources: Vec::new(),
            dest: dest.into(),
            sha1: None,
            size: None,
        }
    }

    /// 附加期望 sha1。
    pub fn with_sha1(mut self, sha1: impl Into<String>) -> Self {
        self.sha1 = Some(sha1.into());
        self
    }

    /// 附加期望大小（字节）。
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// 设置该任务独有的候选源，按传入顺序尝试。
    pub fn with_sources(mut self, sources: impl IntoIterator<Item = MirrorSource>) -> Self {
        self.task_sources = sources.into_iter().collect();
        self
    }

    /// 把上游清单提供的候选 URL 转换为任务级 [`MirrorSource::Provided`]，并保持原顺序。
    pub fn with_urls<I, U>(mut self, urls: I) -> Self
    where
        I: IntoIterator<Item = U>,
        U: Into<String>,
    {
        self.task_sources = urls
            .into_iter()
            .map(|url| MirrorSource::Provided(url.into()))
            .collect();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_sets_optional_fields() {
        let task = DownloadTask::new("https://host/a.jar", "C:/data/a.jar")
            .with_sha1("ABCDEF")
            .with_size(1024);
        assert_eq!(task.url, "https://host/a.jar");
        assert_eq!(task.dest, PathBuf::from("C:/data/a.jar"));
        assert_eq!(task.sha1.as_deref(), Some("ABCDEF"));
        assert_eq!(task.size, Some(1024));
        assert!(task.task_sources.is_empty());
    }

    #[test]
    fn new_leaves_contract_empty() {
        let task = DownloadTask::new("https://host/b", "b");
        assert!(task.sha1.is_none());
        assert!(task.size.is_none());
        assert!(task.task_sources.is_empty());
    }

    #[test]
    fn with_urls_builds_ordered_provided_sources() {
        let task = DownloadTask::new("https://manifest.example/file", "file")
            .with_urls(["https://cdn-a.example/file", "https://cdn-b.example/file"]);

        assert_eq!(
            task.task_sources,
            vec![
                MirrorSource::Provided("https://cdn-a.example/file".into()),
                MirrorSource::Provided("https://cdn-b.example/file".into()),
            ]
        );
    }

    #[test]
    fn with_sources_accepts_mixed_task_level_policy() {
        let task = DownloadTask::new("https://libraries.minecraft.net/file", "file")
            .with_sources([MirrorSource::BmclApi, MirrorSource::Official]);

        assert_eq!(
            task.task_sources,
            vec![MirrorSource::BmclApi, MirrorSource::Official]
        );
    }
}
