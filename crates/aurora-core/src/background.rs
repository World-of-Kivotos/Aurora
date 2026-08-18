//! 自定义背景的图库管理。
//!
//! 玩家选中的图会被复制进 `<数据目录>/backgrounds/`，配置里只记文件名。这样做不是为了整齐：
//! 原图常在下载文件夹或 U 盘里，随时会被清掉或拔走；复制进来之后整个 Aurora 文件夹搬到
//! 另一台机器，背景照样在——这与数据目录便携优先是同一套取舍。
//!
//! 导入时统一转成 JPEG 并压到 [`DISPLAY_WIDTH`] 宽。背景铺在最底层、不需要透明通道，
//! 而一张 4K 原图每次重绘都要 WebView 解码一遍；转码后一张图只有几百 KB，
//! 玩家攒十几张也不占地方。
//!
//! 导入过的图不会因为切换而删除——它们留在目录里构成图库，设置页列出来点一下就换。
//! 「多张背景」因此不需要任何轮换逻辑。

use std::path::{Path, PathBuf};

use image::ImageReader;
use image::imageops::FilterType;
use serde::Serialize;

use crate::config::{BackgroundRef, MAX_BACKGROUND_VEIL, PlateZone};
use crate::error::{CoreError, Result};
use crate::facade::Aurora;

/// 图库目录名，位于数据目录下。
pub const BACKGROUNDS_DIR: &str = "backgrounds";

/// 展示副本的最大宽度。1920 足够铺满任何常见窗口，再大只是让 WebView 多解码像素。
pub const DISPLAY_WIDTH: u32 = 1920;

/// 转码 JPEG 的质量。88 在照片上肉眼已看不出损失，体积却只有无损的零头。
const JPEG_QUALITY: u8 = 88;

/// 图库里的一张图。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackgroundEntry {
    /// 文件名，同时是它在协议 URL 里的路径段。
    pub file: String,
    /// 展示副本的像素尺寸。
    pub width: u32,
    pub height: u32,
    /// 文件字节数，让玩家自己判断该不该清理。
    pub bytes: u64,
    /// 是否为当前正在使用的那张。
    pub is_current: bool,
}

impl Aurora {
    /// 图库目录。不保证存在——列举时缺失即当作空图库，导入时才创建。
    pub fn backgrounds_dir(&self) -> PathBuf {
        self.data_dir().join(BACKGROUNDS_DIR)
    }

    /// 列出图库里的图，按文件名排序。目录不存在返回空表。
    pub async fn list_backgrounds(&self) -> Result<Vec<BackgroundEntry>> {
        let dir = self.backgrounds_dir();
        let current = self
            .config()
            .appearance
            .background
            .as_ref()
            .map(|b| b.file.clone());

        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            // 还没导入过任何图，不是错误。
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(CoreError::BackgroundIo { path: dir, source });
            }
        };

        let mut out = Vec::new();
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| CoreError::BackgroundIo {
                    path: dir.clone(),
                    source,
                })?
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(file) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            // 尺寸只读文件头，不解码整张图：图库页要列十几张，逐张全解码会卡住界面。
            let Ok((width, height)) = ImageReader::open(&path)
                .and_then(|r| r.with_guessed_format())
                .map_err(|_| ())
                .and_then(|r| r.into_dimensions().map_err(|_| ()))
            else {
                // 目录里混进了非图片文件（玩家手动丢进来的），跳过而不是让整个列表失败。
                continue;
            };
            let bytes = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
            out.push(BackgroundEntry {
                is_current: current.as_deref() == Some(file.as_str()),
                file,
                width,
                height,
                bytes,
            });
        }
        out.sort_by(|a, b| a.file.cmp(&b.file));
        Ok(out)
    }

    /// 把一张外部图片导入图库，返回可直接写进配置的引用。不改变当前背景。
    ///
    /// 解码与缩放是 CPU 密集的同步活，扔到阻塞线程池，避免把异步运行时的工作线程占死。
    pub async fn import_background(&self, source: impl AsRef<Path>) -> Result<BackgroundRef> {
        let source = source.as_ref().to_path_buf();
        let dir = self.backgrounds_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|source| CoreError::BackgroundIo {
                path: dir.clone(),
                source,
            })?;

        let stem = source
            .file_stem()
            .and_then(|s| s.to_str())
            .map(sanitize_stem)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "背景".to_owned());

        tokio::task::spawn_blocking(move || import_blocking(&source, &dir, &stem)).await?
    }

    /// 切换当前背景。传 `None` 回到纯纸面。
    ///
    /// 指定的文件必须已在图库里——允许写入一个不存在的文件名，等于让界面拿着断链的引用去加载。
    pub fn set_background(&mut self, file: Option<String>) -> Result<()> {
        let Some(file) = file else {
            self.config_mut().appearance.background = None;
            return Ok(());
        };
        let path = resolve_in_library(&self.backgrounds_dir(), &file)?;
        if !path.is_file() {
            return Err(CoreError::BackgroundNotFound { file });
        }
        // 平均色随图走，切换时重算：沿用上一张的 tint 会在加载间隙闪一下完全无关的颜色。
        // 右下角取样一并重算，顺带把本功能上线前导入、plate 还是 None 的老图补上——
        // 玩家重新选一次就自愈，不需要单独的迁移步骤。
        let decoded = decode_image(&path)?;
        self.config_mut().appearance.background = Some(BackgroundRef {
            file,
            tint: tint_of(&decoded),
            plate: Some(plate_zone_of(&decoded)),
        });
        Ok(())
    }

    /// 给当前背景补上缺失的取样数据，返回是否真的补了（调用方据此决定要不要落盘）。
    ///
    /// 本功能上线前导入的图，配置里没有 plate，前端遇到 None 会退回纸片。原打算靠
    /// 「玩家重新选一次图」顺带补上，但那不成立：没人会为了一个自己都不知道的功能去点那一下，
    /// 结果就是老用户永远看不到这个特性。所以改成读取外观时按需补一次。
    ///
    /// 只在缺失时解码，补完落盘，之后再不触发，稳态下零开销。
    pub fn backfill_plate_zone(&mut self) -> bool {
        let Some(bg) = self.config().appearance.background.as_ref() else {
            return false;
        };
        if bg.plate.is_some() {
            return false;
        }
        let file = bg.file.clone();

        // 取不到图就维持 None，让前端继续用纸片那条兜底，而不是把整个外观读取搞失败——
        // 这是应用外壳的渲染路径，一张壁纸读不出来不该让界面失去背景设置。
        // 但也不能无声吞掉：记一条 warn，排查时能看见。
        let path = match resolve_in_library(&self.backgrounds_dir(), &file) {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!(%file, %err, "补取样时解析图库路径失败，维持未量状态");
                return false;
            }
        };
        let decoded = match decode_image(&path) {
            Ok(decoded) => decoded,
            Err(err) => {
                tracing::warn!(%file, %err, "补取样时解码失败，维持未量状态");
                return false;
            }
        };

        let zone = plate_zone_of(&decoded);
        if let Some(bg) = self.config_mut().appearance.background.as_mut() {
            bg.plate = Some(zone);
        }
        true
    }

    /// 从图库删掉一张图。删的是当前那张时顺带回到纯纸面。
    pub async fn remove_background(&mut self, file: &str) -> Result<()> {
        let path = resolve_in_library(&self.backgrounds_dir(), file)?;
        tokio::fs::remove_file(&path)
            .await
            .map_err(|source| CoreError::BackgroundIo { path, source })?;
        if self
            .config()
            .appearance
            .background
            .as_ref()
            .map(|b| b.file.as_str())
            == Some(file)
        {
            self.config_mut().appearance.background = None;
        }
        Ok(())
    }

    /// 设置纸色遮罩强度，超出上限即钳到上限。
    pub fn set_background_veil(&mut self, veil: u8) {
        self.config_mut().appearance.background_veil = veil.min(MAX_BACKGROUND_VEIL);
    }

    /// 把图库里某张图的字节读出来，供自定义协议直接吐给 WebView。
    ///
    /// 文件名来自 URL，必须经 [`resolve_in_library`] 校验——这是唯一一处外部输入能触碰路径的地方。
    pub async fn read_background(&self, file: &str) -> Result<Vec<u8>> {
        let path = resolve_in_library(&self.backgrounds_dir(), file)?;
        tokio::fs::read(&path)
            .await
            .map_err(|source| CoreError::BackgroundIo { path, source })
    }
}

/// 把一个来自外部的文件名解析成图库内的路径，越界一律拒绝。
///
/// 文件名会经由协议 URL 从 WebView 传进来，等同于不可信输入。只接受单一路径段：
/// 带分隔符、带 `..`、或本身就是绝对路径的，都会被挡在这里——放行任何一种，
/// 这个协议就成了任意文件读取的口子。
fn resolve_in_library(dir: &Path, file: &str) -> Result<PathBuf> {
    let mut parts = Path::new(file).components();
    let only = parts.next();
    let extra = parts.next();
    match (only, extra) {
        (Some(std::path::Component::Normal(name)), None) => Ok(dir.join(name)),
        _ => Err(CoreError::BackgroundName {
            file: file.to_owned(),
        }),
    }
}

/// 把原文件名主干清洗成可安全落盘的名字：去掉路径分隔符与 Windows 保留字符。
fn sanitize_stem(stem: &str) -> String {
    stem.chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_owned()
}

/// 在图库目录里挑一个没被占用的名字：`雪山.jpg`、`雪山-2.jpg`……
///
/// 用可读的原名而不是内容哈希：设置页会把文件名显示给玩家，
/// 一屏 `a3f9c1e2.jpg` 谁也认不出哪张是哪张。
fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.jpg"));
    if !first.exists() {
        return first;
    }
    for n in 2u32.. {
        let candidate = dir.join(format!("{stem}-{n}.jpg"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("2..u32::MAX 内必有空位")
}

/// 导入的同步实现：解码、按需缩放、转 JPEG 落盘、算平均色。
fn import_blocking(source: &Path, dir: &Path, stem: &str) -> Result<BackgroundRef> {
    let decoded = ImageReader::open(source)
        .map_err(|err| CoreError::BackgroundIo {
            path: source.to_path_buf(),
            source: err,
        })?
        .with_guessed_format()
        .map_err(|err| CoreError::BackgroundIo {
            path: source.to_path_buf(),
            source: err,
        })?
        .decode()
        .map_err(|err| CoreError::BackgroundDecode {
            path: source.to_path_buf(),
            reason: err.to_string(),
        })?;

    // 只缩不放：比 1920 窄的图放大只会糊，原样收下即可。
    let display = if decoded.width() > DISPLAY_WIDTH {
        let height =
            (decoded.height() as u64 * DISPLAY_WIDTH as u64 / decoded.width() as u64).max(1);
        decoded.resize(DISPLAY_WIDTH, height as u32, FilterType::Lanczos3)
    } else {
        decoded
    };

    let target = unique_path(dir, stem);
    let rgb = display.to_rgb8();
    // 先编码进内存再整体落盘：直接把 File 交给编码器的话，缓冲要等编码器析构才 flush，
    // 而下一行就要把文件读回来算平均色——那时它还是空的。一张 1920 宽的 JPEG 只有几百 KB。
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|err| CoreError::BackgroundDecode {
            path: target.clone(),
            reason: err.to_string(),
        })?;
    std::fs::write(&target, &buf).map_err(|err| CoreError::BackgroundIo {
        path: target.clone(),
        source: err,
    })?;

    // 从落盘后的 JPEG 重算而不是拿内存里那份：转码是有损的，两处各算各的会差出一两个色阶，
    // 于是同一张图在「刚导入」和「重新选中」时兜底色不同。tint 只有一个定义——磁盘上那张图的平均色。
    // 右下角取样同理：字色要跟玩家真正看到的那张图对齐，而不是转码前的原图。
    let stored = decode_image(&target)?;
    let file = target
        .file_name()
        .and_then(|n| n.to_str())
        .expect("刚写出的文件名由本函数拼出，必为合法 UTF-8")
        .to_owned();
    Ok(BackgroundRef {
        file,
        tint: tint_of(&stored),
        plate: Some(plate_zone_of(&stored)),
    })
}

/// 读一张图并解码。
fn decode_image(path: &Path) -> Result<image::DynamicImage> {
    ImageReader::open(path)
        .map_err(|source| CoreError::BackgroundIo {
            path: path.to_path_buf(),
            source,
        })?
        .with_guessed_format()
        .map_err(|source| CoreError::BackgroundIo {
            path: path.to_path_buf(),
            source,
        })?
        .decode()
        .map_err(|source| CoreError::BackgroundDecode {
            path: path.to_path_buf(),
            reason: source.to_string(),
        })
}

/// 图的平均色，`#rrggbb`。
///
/// 先缩到 8x8 再平均：直接逐像素累加一张 1920x1080 是两百万次加法，
/// 而这个值只用来当加载前的兜底底色，精度到这一步早就够了。
fn tint_of(image: &image::DynamicImage) -> String {
    let small = image.resize_exact(8, 8, FilterType::Triangle).to_rgb8();
    let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
    for px in small.pixels() {
        r += px[0] as u32;
        g += px[1] as u32;
        b += px[2] as u32;
    }
    let n = small.pixels().len() as u32;
    format!("#{:02x}{:02x}{:02x}", r / n, g / n, b / n)
}

/// 右下角信息区在图上的取样范围，从右下角起算的宽、高比例。
///
/// 这两个数对着实际那块内容的尺寸来：它约占内容区的 24% 宽、29% 高，
/// 经 object-fit: cover 折算回图上再留一点富余，就是下面这对值。
///
/// 取样区必须贴着实际显示的那块，不能图省事取一大片：取大了会把根本没有字压着的
/// 图也算进来，一角亮一角暗的图会因此被判成「花」而白白退回纸片。
///
/// 仍是近似——cover 的裁切量随窗口长宽比变化，这里按常见的横向窗口取一个固定值。
/// 极端比例（竖图配宽窗）下量到的与实际显示的会有偏差，那种情况本就不该指望自动配色，
/// 两端分位数过不了准入线，自然退回纸片。
const PLATE_ZONE_W: f32 = 0.28;
const PLATE_ZONE_H: f32 = 0.34;

/// sRGB 分量到线性值的查表，算 WCAG 相对亮度用。
///
/// 逐像素做 powf 太贵——取样区在 1920 宽的图上是几十万像素，而分量只有 256 种取值，
/// 打表把幂运算从「每像素三次」压到「每张图 256 次」。
fn srgb_linear_table() -> [f32; 256] {
    let mut table = [0f32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *slot = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
    }
    table
}

/// 量右下角那块区域的亮度均值与离散度。
fn plate_zone_of(image: &image::DynamicImage) -> PlateZone {
    let rgb = image.to_rgb8();
    let (w, h) = rgb.dimensions();
    // saturating_sub 兜住 1x1 这类极小图：算出来的起点不会越过图的边界。
    let x0 = w.saturating_sub(((w as f32) * PLATE_ZONE_W).ceil() as u32);
    let y0 = h.saturating_sub(((h as f32) * PLATE_ZONE_H).ceil() as u32);

    let table = srgb_linear_table();
    // 直方图而不是把每个亮度值收进 Vec 再排序：分位数只需要计数，
    // 256 个桶对「挑字色」这个用途绰绰有余，也免掉几十万元素的分配与排序。
    let mut hist = [0u32; 256];
    let mut count = 0u32;
    for y in y0..h {
        for x in x0..w {
            let px = rgb.get_pixel(x, y).0;
            let lin = 0.2126 * table[px[0] as usize]
                + 0.7152 * table[px[1] as usize]
                + 0.0722 * table[px[2] as usize];
            let bucket = (lin * 255.0).round().clamp(0.0, 255.0) as usize;
            hist[bucket] += 1;
            count += 1;
        }
    }

    if count == 0 {
        // 空区域只可能来自尺寸为 0 的图，那种图根本进不了图库。给一对横跨全程的分位数，
        // 让前端两种字色都判不达标、老实退回纸片，而不是随便挑一个。
        return PlateZone { p10: 0, p90: 255 };
    }

    PlateZone {
        p10: percentile(&hist, count, 0.10),
        p90: percentile(&hist, count, 0.90),
    }
}

/// 从亮度直方图里取分位数。
fn percentile(hist: &[u32; 256], count: u32, q: f32) -> u8 {
    // ceil 保证 q=0.9 时取的是「至少覆盖 90%」的那个桶，避免整图纯色时因为
    // 目标计数落在 0 而直接返回 0 桶。
    let target = ((count as f32) * q).ceil().max(1.0) as u32;
    let mut seen = 0u32;
    for (bucket, n) in hist.iter().enumerate() {
        seen += n;
        if seen >= target {
            return bucket as u8;
        }
    }
    255
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;
    use image::{Rgb, RgbImage};

    fn aurora_at(data_dir: &Path) -> Aurora {
        Aurora::for_test(
            AuroraConfig::default(),
            data_dir.to_path_buf(),
            data_dir.join(".minecraft"),
        )
    }

    /// 断言 `#rrggbb` 与期望色每通道相差不超过 2。
    ///
    /// 不写死精确值：tint 是从转码后的 JPEG 算的，编码器实现微调就会差一个色阶。
    /// 要验的是「算出来的确实是这张图的主色」，而不是某个版本的编码器输出。
    fn assert_tint_near(actual: &str, expected: [u8; 3]) {
        let hex = actual.strip_prefix('#').expect("tint 应以 # 开头");
        assert_eq!(hex.len(), 6, "tint 应为 6 位十六进制：{actual}");
        for (i, want) in expected.iter().enumerate() {
            let got = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("十六进制通道");
            let diff = got.abs_diff(*want);
            assert!(
                diff <= 2,
                "{actual} 的第 {i} 个通道是 {got}，期望接近 {want}"
            );
        }
    }

    /// 造一张上下两半异色的图：上半 `top`、下半 `bottom`。
    fn split_image(w: u32, h: u32, top: [u8; 3], bottom: [u8; 3]) -> image::DynamicImage {
        let mut img = RgbImage::new(w, h);
        for (_, y, px) in img.enumerate_pixels_mut() {
            *px = Rgb(if y < h / 2 { top } else { bottom });
        }
        image::DynamicImage::ImageRgb8(img)
    }

    fn solid_image(w: u32, h: u32, color: [u8; 3]) -> image::DynamicImage {
        let mut img = RgbImage::new(w, h);
        for px in img.pixels_mut() {
            *px = Rgb(color);
        }
        image::DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn plate_zone_reads_luminance_of_solid_images() {
        // 纯色图的两个分位数必须重合并贴到对应端点。
        let black = plate_zone_of(&solid_image(200, 200, [0, 0, 0]));
        assert_eq!((black.p10, black.p90), (0, 0));

        let white = plate_zone_of(&solid_image(200, 200, [255, 255, 255]));
        assert_eq!((white.p10, white.p90), (255, 255));

        // 中灰按 WCAG 相对亮度算要比按 sRGB 数值算暗得多（伽马）：sRGB 128 的相对亮度
        // 约 0.2159，即 55/255，而不是 128。断言这一点等于断言我们确实做了线性化，
        // 而不是直接平均 sRGB 分量——去掉查表里的 powf 这条就会挂。
        let mid = plate_zone_of(&solid_image(200, 200, [128, 128, 128]));
        assert!(
            (50..=60).contains(&mid.p10) && mid.p10 == mid.p90,
            "sRGB 128 应落在 55 附近且纯色两端重合，实际 p10={} p90={}",
            mid.p10,
            mid.p90
        );
    }

    /// 取样区必须只看右下角。
    ///
    /// 把 plate_zone_of 里的区域起点改成 0（即全图取样），这个断言就会挂：
    /// 全图会同时含黑与白，而右下角实际是纯白。
    #[test]
    fn plate_zone_samples_only_bottom_right() {
        let img = split_image(200, 200, [0, 0, 0], [255, 255, 255]);
        let zone = plate_zone_of(&img);
        assert_eq!(
            (zone.p10, zone.p90),
            (255, 255),
            "取样区应完全落在下半的白色里"
        );
    }

    /// 明暗各半的角落：两端必须分别贴到黑与白，这正是要退回纸片的那类图。
    #[test]
    fn plate_zone_reports_both_ends_on_split_backdrop() {
        // 取样区是底部 45%，让分界落在图高的 80% 处，取样区内就同时含黑与白。
        let (w, h) = (200u32, 200u32);
        let mut img = RgbImage::new(w, h);
        for (_, y, px) in img.enumerate_pixels_mut() {
            *px = Rgb(if y < (h * 4) / 5 {
                [0, 0, 0]
            } else {
                [255, 255, 255]
            });
        }
        let zone = plate_zone_of(&image::DynamicImage::ImageRgb8(img));
        assert_eq!(zone.p10, 0, "偏暗那端应落在黑上");
        assert_eq!(zone.p90, 255, "偏亮那端应落在白上");
    }

    /// 少量高光不该把整块区域判死——用分位数而不是最大/最小值的理由。
    #[test]
    fn plate_zone_ignores_sparse_outliers() {
        let (w, h) = (200u32, 200u32);
        let mut img = RgbImage::new(w, h);
        for px in img.pixels_mut() {
            *px = Rgb([0, 0, 0]);
        }
        // 在取样区里点上约 2% 的纯白像素，远低于 p90 的 10% 门槛。
        for y in (h / 2)..h {
            for x in (w / 2)..w {
                if (x + y) % 50 == 0 {
                    img.put_pixel(x, y, Rgb([255, 255, 255]));
                }
            }
        }
        let zone = plate_zone_of(&image::DynamicImage::ImageRgb8(img));
        assert_eq!(zone.p90, 0, "稀疏高光应被分位数滤掉，实际 p90={}", zone.p90);
    }

    /// 老配置（plate 为 None）必须能在不重新选图的前提下自动补上。
    #[tokio::test]
    async fn backfill_fills_missing_plate_zone() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let mut aurora = aurora_at(tmp.path());
        let dir = aurora.backgrounds_dir();
        std::fs::create_dir_all(&dir).expect("建图库目录");
        // 亮图：补完之后取样值应当偏亮。
        write_solid(&dir, "亮.jpg", 300, 300, [240, 240, 240]);

        // 模拟旧版本写下的配置：有 background，但没量过。
        aurora.config_mut().appearance.background = Some(BackgroundRef {
            file: "亮.jpg".to_owned(),
            tint: "#f0f0f0".to_owned(),
            plate: None,
        });

        assert!(aurora.backfill_plate_zone(), "缺失时应当补上并返回 true");
        let plate = aurora
            .config()
            .appearance
            .background
            .as_ref()
            .and_then(|b| b.plate.clone())
            .expect("补完必须有值");
        assert!(plate.p10 > 200, "亮图的偏暗端也该偏亮，实际 {}", plate.p10);

        // 幂等：已经量过就不再动，避免每次读取外观都白解码一次并反复落盘。
        assert!(!aurora.backfill_plate_zone(), "已有取样时应当返回 false");
    }

    /// 图丢了不该把外观读取拖垮，只维持未量状态。
    #[tokio::test]
    async fn backfill_tolerates_missing_file() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let mut aurora = aurora_at(tmp.path());
        aurora.config_mut().appearance.background = Some(BackgroundRef {
            file: "并不存在.jpg".to_owned(),
            tint: "#000000".to_owned(),
            plate: None,
        });
        assert!(
            !aurora.backfill_plate_zone(),
            "取不到图应当返回 false 而不是 panic"
        );
        assert!(
            aurora
                .config()
                .appearance
                .background
                .as_ref()
                .expect("背景仍在")
                .plate
                .is_none(),
            "补不上时应维持 None，而不是写入一个编出来的值"
        );
    }

    #[test]
    fn import_records_plate_zone() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let dir = tmp.path();
        // 上黑下白：导入后落盘的 JPEG 右下角仍应是白的。
        let src = dir.join("split.png");
        split_image(400, 400, [0, 0, 0], [255, 255, 255])
            .save(&src)
            .expect("写测试图");

        let out = dir.join("out");
        std::fs::create_dir_all(&out).expect("建目录");
        let refe = import_blocking(&src, &out, "split").expect("导入");
        let plate = refe.plate.expect("导入必须量出取样区");
        assert!(
            plate.p10 > 230,
            "右下角是白的，偏暗那端也应接近满值，实际 p10={}",
            plate.p10
        );
    }

    /// 造一张纯色图落盘，返回路径。
    fn write_solid(dir: &Path, name: &str, w: u32, h: u32, color: [u8; 3]) -> PathBuf {
        let mut img = RgbImage::new(w, h);
        for px in img.pixels_mut() {
            *px = Rgb(color);
        }
        let path = dir.join(name);
        img.save(&path).expect("写测试图");
        path
    }

    #[test]
    fn library_path_rejects_traversal_and_separators() {
        let dir = Path::new("/data/backgrounds");
        assert!(resolve_in_library(dir, "雪山.jpg").is_ok());
        // 以下每一条放行都等于把协议变成任意文件读取。
        for bad in [
            "../config.json",
            "..",
            "sub/雪山.jpg",
            "sub\\雪山.jpg",
            "/etc/passwd",
            "C:\\Windows\\win.ini",
            "",
        ] {
            assert!(resolve_in_library(dir, bad).is_err(), "{bad} 不该通过校验");
        }
    }

    #[test]
    fn library_path_stays_inside_dir() {
        let dir = Path::new("/data/backgrounds");
        let got = resolve_in_library(dir, "雪山.jpg").expect("合法文件名");
        assert_eq!(got, dir.join("雪山.jpg"));
    }

    #[test]
    fn sanitize_strips_reserved_characters() {
        assert_eq!(sanitize_stem("雪山:壁纸*2"), "雪山壁纸2");
        assert_eq!(sanitize_stem("  留白  "), "留白");
        assert_eq!(sanitize_stem("a/b\\c"), "abc");
        // 全是保留字符时清空，调用方会回落到默认名。
        assert_eq!(sanitize_stem("///"), "");
    }

    #[test]
    fn unique_path_avoids_collision() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let dir = tmp.path();
        let first = unique_path(dir, "雪山");
        assert_eq!(first.file_name().unwrap(), "雪山.jpg");
        std::fs::write(&first, b"x").expect("占位");
        let second = unique_path(dir, "雪山");
        assert_eq!(second.file_name().unwrap(), "雪山-2.jpg");
        std::fs::write(&second, b"x").expect("占位");
        assert_eq!(unique_path(dir, "雪山").file_name().unwrap(), "雪山-3.jpg");
    }

    #[test]
    fn tint_reads_dominant_color() {
        let mut img = RgbImage::new(4, 4);
        for px in img.pixels_mut() {
            *px = Rgb([200, 40, 30]);
        }
        let tint = tint_of(&image::DynamicImage::ImageRgb8(img));
        assert_eq!(tint, "#c8281e");
    }

    #[tokio::test]
    async fn import_downscales_wide_images_and_keeps_narrow_ones() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let aurora = aurora_at(tmp.path());
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).expect("源目录");

        // 3840 宽应被压到 DISPLAY_WIDTH，且高度按比例走（3840x2160 -> 1920x1080）。
        let wide = write_solid(&src_dir, "wide.png", 3840, 2160, [10, 120, 200]);
        let imported = aurora.import_background(&wide).await.expect("导入宽图");
        let stored = aurora.backgrounds_dir().join(&imported.file);
        let (w, h) = ImageReader::open(&stored)
            .expect("打开")
            .with_guessed_format()
            .expect("识别")
            .into_dimensions()
            .expect("尺寸");
        assert_eq!((w, h), (DISPLAY_WIDTH, 1080));

        // 比上限窄的图不放大。
        let narrow = write_solid(&src_dir, "narrow.png", 800, 600, [10, 120, 200]);
        let imported = aurora.import_background(&narrow).await.expect("导入窄图");
        let stored = aurora.backgrounds_dir().join(&imported.file);
        let (w, h) = ImageReader::open(&stored)
            .expect("打开")
            .with_guessed_format()
            .expect("识别")
            .into_dimensions()
            .expect("尺寸");
        assert_eq!((w, h), (800, 600));
    }

    #[tokio::test]
    async fn import_transcodes_to_jpeg_and_records_tint() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let aurora = aurora_at(tmp.path());
        let src = write_solid(tmp.path(), "雪山.png", 64, 48, [200, 40, 30]);

        let imported = aurora.import_background(&src).await.expect("导入");
        // 统一转 JPEG：源是 png，落盘必须是 jpg。
        assert_eq!(imported.file, "雪山.jpg");
        assert_tint_near(&imported.tint, [200, 40, 30]);
        let stored = aurora.backgrounds_dir().join(&imported.file);
        assert!(stored.is_file());
        // 原图纹丝不动。
        assert!(src.is_file());
    }

    #[tokio::test]
    async fn set_background_rejects_files_outside_library() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let mut aurora = aurora_at(tmp.path());
        // 图库里没有这张。
        assert!(
            aurora
                .set_background(Some("不存在.jpg".to_owned()))
                .is_err()
        );
        // 越界文件名同样拒绝，且不该因为「文件确实存在」而放行。
        assert!(
            aurora
                .set_background(Some("../config.json".to_owned()))
                .is_err()
        );
        assert!(aurora.config().appearance.background.is_none());
    }

    #[tokio::test]
    async fn set_background_switches_and_clears() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let mut aurora = aurora_at(tmp.path());
        let red = write_solid(tmp.path(), "红.png", 32, 32, [200, 40, 30]);
        let blue = write_solid(tmp.path(), "蓝.png", 32, 32, [20, 60, 180]);
        let red = aurora.import_background(&red).await.expect("导入红");
        let blue = aurora.import_background(&blue).await.expect("导入蓝");

        aurora.set_background(Some(red.file.clone())).expect("设红");
        let now = aurora
            .config()
            .appearance
            .background
            .as_ref()
            .expect("有背景");
        assert_eq!(now.file, red.file);
        assert_tint_near(&now.tint, [200, 40, 30]);

        // 切换必须重算 tint，否则加载间隙会闪上一张的颜色。
        aurora
            .set_background(Some(blue.file.clone()))
            .expect("设蓝");
        let now = aurora
            .config()
            .appearance
            .background
            .as_ref()
            .expect("有背景");
        assert_eq!(now.file, blue.file);
        assert_tint_near(&now.tint, [20, 60, 180]);

        // 同一张图，导入时与重新选中时算出的兜底色必须一致——两处各算各的会差色阶。
        assert_eq!(now.tint, blue.tint);

        aurora.set_background(None).expect("清除");
        assert!(aurora.config().appearance.background.is_none());
    }

    #[tokio::test]
    async fn list_reports_current_and_skips_non_images() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let mut aurora = aurora_at(tmp.path());
        let a = write_solid(tmp.path(), "a.png", 40, 30, [10, 20, 30]);
        let b = write_solid(tmp.path(), "b.png", 60, 40, [90, 80, 70]);
        let a = aurora.import_background(&a).await.expect("导入 a");
        aurora.import_background(&b).await.expect("导入 b");
        // 玩家手动往目录里丢的杂物不该让整个列表失败。
        std::fs::write(aurora.backgrounds_dir().join("说明.txt"), b"hello").expect("杂物");

        aurora
            .set_background(Some(a.file.clone()))
            .expect("设为当前");
        let listed = aurora.list_backgrounds().await.expect("列举");
        assert_eq!(listed.len(), 2, "非图片应被跳过");
        assert_eq!(listed[0].file, "a.jpg");
        assert_eq!((listed[0].width, listed[0].height), (40, 30));
        assert!(listed[0].is_current);
        assert!(!listed[1].is_current);
        assert!(listed[0].bytes > 0);
    }

    #[tokio::test]
    async fn list_on_missing_dir_is_empty_not_error() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let aurora = aurora_at(tmp.path());
        assert!(aurora.list_backgrounds().await.expect("空图库").is_empty());
    }

    #[tokio::test]
    async fn remove_deletes_file_and_clears_current() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let mut aurora = aurora_at(tmp.path());
        let src = write_solid(tmp.path(), "x.png", 32, 32, [1, 2, 3]);
        let imported = aurora.import_background(&src).await.expect("导入");
        aurora
            .set_background(Some(imported.file.clone()))
            .expect("设为当前");

        aurora
            .remove_background(&imported.file)
            .await
            .expect("删除");
        assert!(!aurora.backgrounds_dir().join(&imported.file).is_file());
        // 删掉的正是当前那张，配置必须跟着清空，否则界面会拿着断链引用去加载。
        assert!(aurora.config().appearance.background.is_none());
    }

    #[tokio::test]
    async fn remove_other_keeps_current() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let mut aurora = aurora_at(tmp.path());
        let keep = write_solid(tmp.path(), "留.png", 32, 32, [1, 2, 3]);
        let drop = write_solid(tmp.path(), "删.png", 32, 32, [9, 9, 9]);
        let keep = aurora.import_background(&keep).await.expect("导入留");
        let drop = aurora.import_background(&drop).await.expect("导入删");
        aurora
            .set_background(Some(keep.file.clone()))
            .expect("设当前");

        aurora
            .remove_background(&drop.file)
            .await
            .expect("删另一张");
        assert_eq!(
            aurora
                .config()
                .appearance
                .background
                .as_ref()
                .map(|b| b.file.as_str()),
            Some(keep.file.as_str())
        );
    }

    #[tokio::test]
    async fn read_background_serves_bytes_and_blocks_traversal() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let aurora = aurora_at(tmp.path());
        let src = write_solid(tmp.path(), "y.png", 32, 32, [4, 5, 6]);
        let imported = aurora.import_background(&src).await.expect("导入");

        let bytes = aurora
            .read_background(&imported.file)
            .await
            .expect("读字节");
        // JPEG 魔数，确认吐出去的确实是转码后的图。
        assert_eq!(&bytes[..2], &[0xff, 0xd8]);

        // 数据目录里放一个敏感文件，确认协议拿不到它。
        std::fs::write(tmp.path().join("config.json"), b"secret").expect("写配置");
        assert!(aurora.read_background("../config.json").await.is_err());
    }

    #[test]
    fn veil_is_clamped_to_max() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let mut aurora = aurora_at(tmp.path());
        aurora.set_background_veil(30);
        assert_eq!(aurora.config().appearance.background_veil, 30);
        // 越界钳到上限而不是照单全收：再高纸色就把图盖没了。
        aurora.set_background_veil(200);
        assert_eq!(
            aurora.config().appearance.background_veil,
            MAX_BACKGROUND_VEIL
        );
    }
}
