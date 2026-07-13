//! 完整性检查与缺失补全（`ensure_complete`）。
//!
//! 任何版本启动前都要确认「本体 jar / 库 / 资源 / natives」齐备。得益于 aurora-download 引擎的
//! 幂等语义（目标已存在且 sha1/大小吻合即跳过），补全逻辑与全新安装是同一套：把该版本的完整文件
//! 计划重跑一遍，缺什么下什么、坏什么重下什么，已就绪的文件零成本略过。这也是原版安装
//! ([`crate::vanilla`]) 的公共下半段。
//!
//! `target_id` 把「客户端 jar 与 natives 的落点版本」与「被检查的版本 JSON 的 id」解耦：原版安装
//! 时两者相同；加载器版本（已合并解析）则复用其原版的 jar 与 natives 目录，故传原版 id。

use aurora_download::DownloadTask;
use aurora_version::{AssetLayout, AssetObjectsIndex, VersionJson};

use crate::context::InstallContext;
use crate::error::{Error, Result, io_err};
use crate::natives;
use crate::plan;

/// 一次补全的结果计数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionReport {
    /// 是否确保了客户端主 jar（版本 JSON 声明了 downloads.client 才有）。
    pub client_jar: bool,
    /// 确保就绪的库文件数。
    pub libraries: usize,
    /// 确保就绪的资源对象数。
    pub assets: usize,
    /// 解压出的 natives 文件数。
    pub natives: u32,
}

/// 确保 `version` 描述的版本在本地齐备，缺失即补全。
///
/// 前提：`version` 已是「合并解析后」的自洽版本 JSON（无 inheritsFrom）。
pub async fn ensure_complete(
    cx: InstallContext<'_>,
    version: &VersionJson,
    target_id: &str,
) -> Result<CompletionReport> {
    // 第一批：库 + 日志配置 + 客户端 jar + assetIndex（都并入一池）。
    let mut primary = plan::library_tasks(version, cx.runtime, cx.layout)?;
    let libraries = primary.len();

    let client_jar = if let Some(client) = version.downloads.as_ref().and_then(|d| d.client.as_ref())
    {
        primary.push(
            DownloadTask::new(client.url.clone(), cx.layout.version_jar(target_id))
                .with_sha1(client.sha1.clone())
                .with_size(client.size),
        );
        true
    } else {
        false
    };

    if let Some(log) = plan::logging_task(version, cx.layout) {
        primary.push(log);
    }
    let has_assets = version.asset_index.is_some();
    if has_assets {
        primary.push(plan::asset_index_task(version, cx.layout)?);
    }
    cx.run_batch(primary, "补全本体与库", None).await?;

    // 第二批：资源对象全量补全（仅当版本带 assetIndex）。
    let assets = if has_assets {
        let index = read_asset_index(cx, version).await?;
        let object_tasks = plan::asset_object_tasks(&index, cx.layout);
        let count = object_tasks.len();
        cx.run_batch(object_tasks, "补全资源", None).await?;
        materialize_assets(cx, version, &index).await?;
        count
    } else {
        0
    };

    // natives 解压到 target 版本的 natives 目录。
    let natives = natives::extract_all_natives(version, cx.runtime, cx.layout, target_id).await?;

    Ok(CompletionReport {
        client_jar,
        libraries,
        assets,
        natives,
    })
}

/// 从磁盘读回并解析已下载的 assetIndex。
async fn read_asset_index(
    cx: InstallContext<'_>,
    version: &VersionJson,
) -> Result<AssetObjectsIndex> {
    let index_id = version
        .asset_index
        .as_ref()
        .ok_or_else(|| Error::MissingAssetIndex {
            version: version.id.clone(),
        })?
        .id
        .clone();
    let path = cx.layout.asset_index_json(&index_id);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|source| io_err(&path, source))?;
    AssetObjectsIndex::from_json_str(&String::from_utf8_lossy(&bytes)).map_err(Error::from)
}

/// 对 virtual / map_to_resources 布局，把 objects 里的对象按逻辑名物化到展开目录。
async fn materialize_assets(
    cx: InstallContext<'_>,
    version: &VersionJson,
    index: &AssetObjectsIndex,
) -> Result<()> {
    let target_dir = match index.layout() {
        AssetLayout::Standard => return Ok(()),
        AssetLayout::Virtual => {
            let index_id = version
                .asset_index
                .as_ref()
                .map(|a| a.id.clone())
                .or_else(|| version.assets.clone())
                .unwrap_or_default();
            cx.layout.asset_virtual_dir(&index_id)
        }
        AssetLayout::MapToResources => cx.layout.resources_dir(),
    };

    for (logical, object) in &index.objects {
        let source = cx.layout.asset_object_path(&object.hash);
        let dest = target_dir.join(crate::layout::rel_to_path(logical));
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|src| io_err(parent, src))?;
        }
        tokio::fs::copy(&source, &dest)
            .await
            .map_err(|src| io_err(&dest, src))?;
    }
    Ok(())
}
