//! 单文件下载引擎：探测、分块/单流下载、断点续传、合并、校验、退避重试与换源。
//!
//! [`Downloader::download`] 的一次调用完整覆盖：
//! 1. 若目标已存在且 sha1/大小校验通过则直接跳过（可重入、幂等安装）。
//! 2. 优先使用任务级候选源；任务未指定时，再由 [`SourcePlan`] 展开全局候选源列表。
//! 3. 每个源用 [`retry_async`] 做指数退避重试；耗尽后切换下一个源（即「n 次后切换镜像」）。
//! 4. 已知大小且达阈值的大文件走 Range 分块并发下载：先用 [`probe_final_url`] 把跳转解到终点，
//!    再对终点地址发 Range，分片落 `.aurora-partN`；网络中断保留已完成分片供下次断点续传，
//!    损坏（哈希不符）则清分片重下；换源前，没有 sha1 的任务会先清掉本源残留分片（见 `run`）。
//! 5. 分片路径在某个源上彻底失败后自动回退为整文件单请求，宁可失去并行加速也要把文件装上。
//! 6. 合并到同目录临时文件，校验大小与 sha1，最后原子 rename 覆盖目标。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aurora_base::retry::{RetryPolicy, retry_async};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::chunk::{Chunk, ChunkConfig, ChunkPlan};
use crate::error::{Error, Result};
use crate::progress::ProgressReporter;
use crate::source::SourcePlan;
use crate::task::DownloadTask;

/// 下载引擎的运行参数。
#[derive(Clone, Default)]
pub struct DownloadConfig {
    /// 分块下载参数。
    pub chunk: ChunkConfig,
    /// 单个源上的退避重试策略；其 `max_attempts` 即「切换镜像前在同一源上的尝试次数」。
    pub retry: RetryPolicy,
    /// 下载源优先级方案。
    pub sources: SourcePlan,
}

/// 单文件下载引擎。可廉价克隆（内部 `reqwest::Client` 与配置均为 `Arc` 共享），供并发池分发。
#[derive(Clone)]
pub struct Downloader {
    client: reqwest::Client,
    config: Arc<DownloadConfig>,
}

impl Downloader {
    /// 用给定客户端与配置构造。客户端应由 [`aurora_base::http::build_client`] 统一构建。
    pub fn new(client: reqwest::Client, config: DownloadConfig) -> Self {
        Self {
            client,
            config: Arc::new(config),
        }
    }

    /// 用默认配置构造。
    pub fn with_defaults(client: reqwest::Client) -> Self {
        Self::new(client, DownloadConfig::default())
    }

    /// 只读访问运行参数。
    pub fn config(&self) -> &DownloadConfig {
        &self.config
    }

    /// 下载单个文件（无进度上报）。完成即目标文件已就绪并通过完整性校验。
    pub async fn download(&self, task: &DownloadTask) -> Result<()> {
        self.run(task, None).await
    }

    /// 下载单个文件，可选挂接进度累加器。这是引擎的总入口，负责换源编排。
    pub(crate) async fn run(
        &self,
        task: &DownloadTask,
        progress: Option<&ProgressReporter>,
    ) -> Result<()> {
        if self.already_complete(task).await? {
            tracing::debug!(dest = %task.dest.display(), "目标文件已存在且校验通过，跳过下载");
            return Ok(());
        }

        let candidates = if task.task_sources.is_empty() {
            self.config.sources.candidates(&task.url)?
        } else {
            self.config
                .sources
                .candidates_from(&task.url, &task.task_sources)?
        };
        let mut last_err: Option<Error> = None;
        for (index, url) in candidates.iter().enumerate() {
            let chunked = wants_chunking(&self.config.chunk, url, task.size);
            // 每个源独立退避重试：retry_async 的 op 每次生成新 future，携带当前源的 URL。
            let result = retry_async(&self.config.retry, || {
                self.attempt(url, task, progress, chunked)
            })
            .await;

            let outcome = match result {
                Ok(()) => Ok(()),
                // 分片路径在本源上彻底失败：退回整文件单请求再试一轮。分片依赖 Range，而 Range
                // 恰是最容易被 CDN 前端搞坏的一环（见 probe_final_url 的注释），宁可失去并行加速，
                // 也不能因此判定文件装不上。
                Err(chunk_err) if chunked => {
                    tracing::info!(
                        file = %task.dest.display(),
                        %url,
                        "分片路径失败，回退整文件单请求重下"
                    );
                    match retry_async(&self.config.retry, || {
                        self.attempt(url, task, progress, false)
                    })
                    .await
                    {
                        Ok(()) => {
                            // 分片路径已放弃，残留的 .aurora-partN 不会再被续传复用，就地清掉。
                            self.cleanup_parts(task).await;
                            Ok(())
                        }
                        Err(fallback_err) => {
                            tracing::warn!(
                                stage = "fallback",
                                file = %task.dest.display(),
                                %url,
                                http_status = ?fallback_err.http_status(),
                                // 分片阶段的状态码单列一格：两个阶段的拒绝理由往往不同
                                // （典型是分片 404、回退 503），只看 http_status 会把前者看丢。
                                chunked_status = ?chunk_err.http_status(),
                                error = %fallback_err,
                                "整文件回退同样失败"
                            );
                            Err(Error::ChunkedFallbackFailed {
                                url: url.clone(),
                                chunked: Box::new(chunk_err),
                                fallback: Box::new(fallback_err),
                            })
                        }
                    }
                }
                Err(err) => Err(err),
            };

            match outcome {
                Ok(()) => return Ok(()),
                Err(err) => {
                    // 换源前的分片纪律：`.aurora-partN` 只按「字节数吻合」判定已完成，与是哪个源
                    // 写的无关。带 sha1 的任务即使跨源拼接出坏文件也会在 finalize 被抓住并连分片
                    // 一起清掉；没有 sha1 的任务只校验总大小，那层兜底不存在，必须在换源前把本源
                    // 残留的分片清掉，否则下一个源会把上一个源写的字节当成已完成直接拼进去。
                    // 只在还有下一个源时清：最后一个源上留着分片，是给下次进程重跑用的断点。
                    if task.sha1.is_none() && index + 1 < candidates.len() {
                        self.cleanup_parts(task).await;
                    }
                    tracing::warn!(
                        source_index = index,
                        file = %task.dest.display(),
                        %url,
                        http_status = ?err.http_status(),
                        error = %err,
                        "该下载源尝试耗尽，切换下一个源"
                    );
                    last_err = Some(err);
                }
            }
        }
        Err(Error::AllSourcesExhausted {
            url: task.url.clone(),
            last: Box::new(last_err.expect("候选源列表非空，循环必产生至少一个错误")),
        })
    }

    /// 针对一个具体 URL 的一次完整下载尝试：下载 -> 合并临时文件 -> 校验 -> 原子落位。
    ///
    /// `chunked` 由调用方决定而非此处推导：回退路径要在同一个 URL 上强制走整文件单请求。
    async fn attempt(
        &self,
        url: &str,
        task: &DownloadTask,
        progress: Option<&ProgressReporter>,
        chunked: bool,
    ) -> Result<()> {
        if let Some(parent) = task.dest.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| io_error(parent, source))?;
        }

        let temp = temp_path(&task.dest);
        let parts = if chunked {
            self.download_chunked(url, task, &temp, progress).await?
        } else {
            self.download_stream(url, &temp, progress).await?;
            Vec::new()
        };

        self.finalize(&temp, task, &parts).await
    }

    /// 单流下载：一次 GET 流式落到临时文件。用于小文件或大小未知的文件。
    async fn download_stream(
        &self,
        url: &str,
        temp: &Path,
        progress: Option<&ProgressReporter>,
    ) -> Result<()> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|source| Error::Request {
                url: url.to_owned(),
                source,
            })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(Error::Status {
                url: url.to_owned(),
                status: status.as_u16(),
            });
        }
        if let Err(err) = stream_to_file(resp, temp, progress).await {
            // 单流不做续传，失败即清掉半截临时文件，下次从头再来。
            let _ = tokio::fs::remove_file(temp).await;
            return Err(err);
        }
        Ok(())
    }

    /// 分块下载的入口：先把跳转解干净，再对最终地址发 Range，失败按阶段落日志。
    async fn download_chunked(
        &self,
        url: &str,
        task: &DownloadTask,
        temp: &Path,
        progress: Option<&ProgressReporter>,
    ) -> Result<Vec<PathBuf>> {
        let final_url = match probe_final_url(&self.client, url).await {
            Ok(resolved) => resolved,
            Err(err) => {
                tracing::warn!(
                    stage = "probe",
                    file = %task.dest.display(),
                    %url,
                    http_status = ?err.http_status(),
                    error = %err,
                    "探测最终下载地址失败"
                );
                return Err(err);
            }
        };
        if final_url != url {
            tracing::debug!(
                file = %task.dest.display(),
                %url,
                final_url = %final_url,
                "下载地址存在跳转，分片请求改打最终地址"
            );
        }

        let result = self.fetch_chunks(&final_url, task, temp, progress).await;
        if let Err(err) = &result {
            tracing::warn!(
                stage = "chunk",
                file = %task.dest.display(),
                %url,
                final_url = %final_url,
                http_status = ?err.http_status(),
                error = %err,
                "分片下载失败"
            );
        }
        result
    }

    /// 分块并发下载。返回本次涉及的分片文件列表（供合并后清理）；整体返回时临时文件已是完整合并结果。
    ///
    /// `url` 必须是 [`probe_final_url`] 解出的最终地址：所有 Range 请求都直接打在源站上，
    /// 不再经过任何 302，否则会踩到 CloudFront「302 + Range 变 404」的坑。
    async fn fetch_chunks(
        &self,
        url: &str,
        task: &DownloadTask,
        temp: &Path,
        progress: Option<&ProgressReporter>,
    ) -> Result<Vec<PathBuf>> {
        let total = task.size.expect("分块下载路径要求已知文件大小");
        let plan = ChunkPlan::compute(
            total,
            self.config.chunk.chunk_size,
            self.config.chunk.max_chunks,
        );
        let first = plan.chunks[0];

        // 首块兼任「探测该源是否支持 Range」之责。
        let resp0 = ranged_get(&self.client, url, first.start, first.end).await?;
        if resp0.status() == reqwest::StatusCode::OK {
            // 服务器忽略了 Range、返回整体 200：此响应即完整文件，直接落到合并临时文件。
            if let Err(err) = stream_to_file(resp0, temp, progress).await {
                let _ = tokio::fs::remove_file(temp).await;
                return Err(err);
            }
            return Ok(Vec::new());
        }

        // 206：写首片。首片每次尝试都重下（一个块尺寸，代价可忽略），确保 Range 支持性判定可靠。
        let part0 = part_path(&task.dest, first.index);
        let written0 = stream_to_file(resp0, &part0, progress).await?;
        if written0 != first.byte_len() {
            let _ = tokio::fs::remove_file(&part0).await;
            return Err(Error::IncompleteBody {
                url: url.to_owned(),
                expected: first.byte_len(),
                actual: written0,
            });
        }

        // 其余分片并发下载，已完成的分片（尺寸吻合）跳过——这是断点续传的落点。
        let semaphore = Arc::new(Semaphore::new(self.config.chunk.chunk_concurrency.max(1)));
        let owned_progress = progress.cloned();
        let mut set: JoinSet<Result<()>> = JoinSet::new();
        for chunk in plan.chunks.iter().skip(1).copied() {
            let part = part_path(&task.dest, chunk.index);
            if part_complete(&part, chunk.byte_len()).await {
                continue;
            }
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("分块信号量未关闭");
            let client = self.client.clone();
            let url = url.to_owned();
            let prog = owned_progress.clone();
            set.spawn(async move {
                let _permit = permit;
                download_one_chunk(&client, &url, chunk, &part, prog.as_ref()).await
            });
        }

        let mut first_err: Option<Error> = None;
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    if first_err.is_none() {
                        first_err = Some(err);
                    }
                }
                Err(join) => {
                    if first_err.is_none() {
                        first_err = Some(Error::ChunkTaskJoin(join));
                    }
                }
            }
        }
        if let Some(err) = first_err {
            // 网络类失败：保留已完成分片，交由外层重试断点续传，不在此清理。
            return Err(err);
        }

        concat_parts(&plan, &task.dest, temp).await?;
        Ok(plan
            .chunks
            .iter()
            .map(|chunk| part_path(&task.dest, chunk.index))
            .collect())
    }

    /// 合并后校验并原子落位。大小/哈希不符视为损坏：清掉临时文件与分片，让外层重下。
    async fn finalize(&self, temp: &Path, task: &DownloadTask, parts: &[PathBuf]) -> Result<()> {
        if let Some(expected) = task.size {
            let meta = tokio::fs::metadata(temp)
                .await
                .map_err(|source| io_error(temp, source))?;
            if meta.len() != expected {
                cleanup(temp, parts).await;
                return Err(Error::SizeMismatch {
                    url: task.url.clone(),
                    expected,
                    actual: meta.len(),
                });
            }
        }
        if let Some(sha1) = &task.sha1
            && let Err(err) = aurora_base::fs::verify_sha1(temp, sha1).await
        {
            // 哈希不符 -> 分片产出了错误内容，必须删除分片，否则续传会一直复用坏数据。
            cleanup(temp, parts).await;
            return Err(err.into());
        }
        tokio::fs::rename(temp, &task.dest)
            .await
            .map_err(|source| io_error(&task.dest, source))?;
        for part in parts {
            let _ = tokio::fs::remove_file(part).await;
        }
        Ok(())
    }

    /// 清掉该任务遗留的全部分片文件。
    ///
    /// 分片计划由「大小 + 分块配置」确定性算出，故无需记账即可反推出所有 `.aurora-partN`。
    /// 仅在放弃分片路径（已改走整文件回退并成功）后调用：这些分片再也不会被续传复用，
    /// 留着只是长期占盘。
    async fn cleanup_parts(&self, task: &DownloadTask) {
        let Some(total) = task.size else {
            return;
        };
        let plan = ChunkPlan::compute(
            total,
            self.config.chunk.chunk_size,
            self.config.chunk.max_chunks,
        );
        for chunk in &plan.chunks {
            let _ = tokio::fs::remove_file(part_path(&task.dest, chunk.index)).await;
        }
    }

    /// 目标是否已存在且满足完整性契约（有 sha1 校 sha1，否则有大小校大小，都没有则不认为完整）。
    async fn already_complete(&self, task: &DownloadTask) -> Result<bool> {
        match tokio::fs::metadata(&task.dest).await {
            Ok(meta) => {
                if let Some(sha1) = &task.sha1 {
                    match aurora_base::fs::verify_sha1(&task.dest, sha1).await {
                        Ok(()) => Ok(true),
                        Err(aurora_base::Error::HashMismatch { .. }) => Ok(false),
                        Err(err) => Err(err.into()),
                    }
                } else if let Some(size) = task.size {
                    Ok(meta.len() == size)
                } else {
                    Ok(false)
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(io_error(&task.dest, source)),
        }
    }
}

/// 该 URL 上的这个任务是否走 Range 分片路径。
///
/// 三个条件缺一不可：分块总开关打开、文件大小已知且达到拆分阈值、host 不在禁用名单里。
fn wants_chunking(config: &ChunkConfig, url: &str, size: Option<u64>) -> bool {
    config.enabled
        && size.is_some_and(|size| size >= config.min_split_size)
        && !config.is_host_excluded(url)
}

/// 探测最终下载地址：发一次不带 Range 的 GET，跟随重定向后取响应的最终 URL。
///
/// 为什么必须先探测：CurseForge 的 `edge.forgecdn.net` 是 CloudFront 前端，它把请求 302 到真正
/// 存放文件的 `mediafilez.forgecdn.net`；分片请求跟着这条 302 走会拿到 404，而同一个 mediafilez
/// 地址直接发 Range 却是正常的 206。坏的是「跟随 302 的 Range 请求」这个组合，因此把跳转
/// 先解干净、让所有分片都打在终点地址上，就绕开了它。多一次往返，换来对任何会跳转的 CDN 都成立。
///
/// 为什么探测本身不带 Range：探测的职责是把跳转链走通，而 Range 恰是会在跳转上出事的那个头。
/// 给探测也带上 Range，遇到「跟随跳转后 Range 就 404」的 CDN 会连地址都解不出来，把本可以靠
/// 终点地址分片下完的文件直接踩进整文件回退。反过来，若某台 CDN 只对带 Range 的请求跳转、
/// 不带 Range 时直接 200，则探测解出的就是原地址、分片仍会撞上那条跳转——那一路由整文件回退
/// 兜住（见 [`Downloader::run`] 的回退分支与 `chunk_failure_falls_back_to_whole_file` 用例），
/// 文件照样装得上。两类 CDN 都有出路，故探测固定用「不带 Range」这一种形状。
///
/// 为什么不会把整个文件拉下来：reqwest 的响应体是惰性流，`send()` 返回时只读完了状态行与响应头，
/// 响应体一个字节都还没被拉取；此处取到最终 URL 后立即 drop 掉 Response，底层传输随之取消。
/// 不用 HEAD 是因为部分 CDN 对 HEAD 直接回 403/405，探测本身就会误判成源不可用。
async fn probe_final_url(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|source| Error::Request {
            url: url.to_owned(),
            source,
        })?;
    let status = resp.status();
    let final_url = resp.url().as_str().to_owned();
    drop(resp);
    if !status.is_success() {
        // 报最终地址而非原地址：跳转链末端才是真正拒绝请求的那一台。
        return Err(Error::Status {
            url: final_url,
            status: status.as_u16(),
        });
    }
    Ok(final_url)
}

/// 发起一次 Range 请求。仅放行 206（分块命中）与 200（服务器忽略 Range 的整体响应）两种成功形态。
async fn ranged_get(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
) -> Result<reqwest::Response> {
    let value = format!("bytes={start}-{end}");
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, value)
        .send()
        .await
        .map_err(|source| Error::Request {
            url: url.to_owned(),
            source,
        })?;
    let status = resp.status();
    if status == reqwest::StatusCode::PARTIAL_CONTENT || status == reqwest::StatusCode::OK {
        Ok(resp)
    } else {
        Err(Error::Status {
            url: url.to_owned(),
            status: status.as_u16(),
        })
    }
}

/// 下载单个非首分片到其分片文件，并校验落盘字节数与区间长度一致。
async fn download_one_chunk(
    client: &reqwest::Client,
    url: &str,
    chunk: Chunk,
    part: &Path,
    progress: Option<&ProgressReporter>,
) -> Result<()> {
    let resp = ranged_get(client, url, chunk.start, chunk.end).await?;
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        // 非零起始的分片却收到 200，意味着该源不支持 Range：上抛以触发换源，而非把整文件塞进某一分片。
        return Err(Error::RangeUnsupported {
            url: url.to_owned(),
        });
    }
    let written = stream_to_file(resp, part, progress).await?;
    if written != chunk.byte_len() {
        let _ = tokio::fs::remove_file(part).await;
        return Err(Error::IncompleteBody {
            url: url.to_owned(),
            expected: chunk.byte_len(),
            actual: written,
        });
    }
    Ok(())
}

/// 流式把响应体写入文件（覆盖），逐段累加进度，写完 fsync。返回写入字节数。
async fn stream_to_file(
    mut resp: reqwest::Response,
    path: &Path,
    progress: Option<&ProgressReporter>,
) -> Result<u64> {
    let url = resp.url().as_str().to_owned();
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|source| io_error(path, source))?;
    let mut written = 0u64;
    loop {
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                file.write_all(&bytes)
                    .await
                    .map_err(|source| io_error(path, source))?;
                written += bytes.len() as u64;
                if let Some(reporter) = progress {
                    reporter.add_bytes(bytes.len() as u64);
                }
            }
            Ok(None) => break,
            Err(source) => return Err(Error::Request { url, source }),
        }
    }
    file.sync_all()
        .await
        .map_err(|source| io_error(path, source))?;
    Ok(written)
}

/// 按序号顺序把分片拼接进合并临时文件，写完 fsync。
async fn concat_parts(plan: &ChunkPlan, dest: &Path, temp: &Path) -> Result<()> {
    let mut out = tokio::fs::File::create(temp)
        .await
        .map_err(|source| io_error(temp, source))?;
    for chunk in &plan.chunks {
        let part = part_path(dest, chunk.index);
        let mut input = tokio::fs::File::open(&part)
            .await
            .map_err(|source| io_error(&part, source))?;
        tokio::io::copy(&mut input, &mut out)
            .await
            .map_err(|source| io_error(&part, source))?;
    }
    out.sync_all()
        .await
        .map_err(|source| io_error(temp, source))?;
    Ok(())
}

/// 分片是否已完整下载：文件存在且尺寸恰等于该分块应有长度。
async fn part_complete(path: &Path, expected: u64) -> bool {
    matches!(tokio::fs::metadata(path).await, Ok(meta) if meta.len() == expected)
}

/// 删除合并临时文件与全部分片（用于损坏后彻底重下）。
async fn cleanup(temp: &Path, parts: &[PathBuf]) {
    let _ = tokio::fs::remove_file(temp).await;
    for part in parts {
        let _ = tokio::fs::remove_file(part).await;
    }
}

/// 把 IO 错误包成携带路径的 crate 错误。
fn io_error(path: &Path, source: std::io::Error) -> Error {
    Error::Base(aurora_base::Error::Io {
        path: path.to_owned(),
        source,
    })
}

/// 在目标路径尾部追加后缀，得到同目录的辅助文件路径（保证 rename 同卷原子）。
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut raw = path.as_os_str().to_os_string();
    raw.push(suffix);
    PathBuf::from(raw)
}

/// 合并临时文件路径：`<dest>.aurora-tmp`。
fn temp_path(dest: &Path) -> PathBuf {
    append_suffix(dest, ".aurora-tmp")
}

/// 分片文件路径：`<dest>.aurora-partN`（确定性命名，供断点续传定位）。
fn part_path(dest: &Path, index: usize) -> PathBuf {
    append_suffix(dest, &format!(".aurora-part{index}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_config() -> ChunkConfig {
        ChunkConfig {
            enabled: true,
            min_split_size: 1024,
            ..ChunkConfig::default()
        }
    }

    #[test]
    fn chunking_requires_known_size_at_or_above_threshold() {
        let config = chunk_config();
        let url = "https://edge.forgecdn.net/files/1/2/big.jar";
        // 大小未知：无从切分，只能单流。
        assert!(!wants_chunking(&config, url, None));
        // 恰好差 1 字节到阈值 -> 不切；恰好达到阈值 -> 切。
        assert!(!wants_chunking(&config, url, Some(1023)));
        assert!(wants_chunking(&config, url, Some(1024)));
        assert!(wants_chunking(&config, url, Some(22 * 1024 * 1024)));
    }

    #[test]
    fn chunking_respects_switch_and_host_exclusion() {
        let disabled = ChunkConfig {
            enabled: false,
            ..chunk_config()
        };
        assert!(!wants_chunking(
            &disabled,
            "https://cdn.modrinth.com/a.jar",
            Some(1 << 20)
        ));

        let excluded = ChunkConfig {
            excluded_hosts: vec!["edge.forgecdn.net".into()],
            ..chunk_config()
        };
        assert!(!wants_chunking(
            &excluded,
            "https://edge.forgecdn.net/files/1/2/big.jar",
            Some(1 << 20)
        ));
        assert!(wants_chunking(
            &excluded,
            "https://cdn.modrinth.com/a.jar",
            Some(1 << 20)
        ));
    }

    #[test]
    fn part_and_temp_paths_are_siblings_of_dest() {
        let dest = Path::new("C:/data/lib.jar");
        assert_eq!(temp_path(dest), PathBuf::from("C:/data/lib.jar.aurora-tmp"));
        assert_eq!(
            part_path(dest, 3),
            PathBuf::from("C:/data/lib.jar.aurora-part3")
        );
    }
}
