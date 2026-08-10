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

use image::imageops::FilterType;
use image::ImageReader;
use serde::Serialize;

use crate::config::{BackgroundRef, MAX_BACKGROUND_VEIL};
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
                return Err(CoreError::BackgroundIo {
                    path: dir,
                    source,
                });
            }
        };

        let mut out = Vec::new();
        while let Some(entry) = entries
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
        let tint = average_color(&path)?;
        self.config_mut().appearance.background = Some(BackgroundRef { file, tint });
        Ok(())
    }

    /// 从图库删掉一张图。删的是当前那张时顺带回到纯纸面。
    pub async fn remove_background(&mut self, file: &str) -> Result<()> {
        let path = resolve_in_library(&self.backgrounds_dir(), file)?;
        tokio::fs::remove_file(&path)
            .await
            .map_err(|source| CoreError::BackgroundIo { path, source })?;
        if self.config().appearance.background.as_ref().map(|b| b.file.as_str()) == Some(file) {
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
        let height = (decoded.height() as u64 * DISPLAY_WIDTH as u64 / decoded.width() as u64).max(1);
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
    let tint = average_color(&target)?;
    let file = target
        .file_name()
        .and_then(|n| n.to_str())
        .expect("刚写出的文件名由本函数拼出，必为合法 UTF-8")
        .to_owned();
    Ok(BackgroundRef { file, tint })
}

/// 读一张图并算平均色。
fn average_color(path: &Path) -> Result<String> {
    let decoded = ImageReader::open(path)
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
        })?;
    Ok(tint_of(&decoded))
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
            assert!(diff <= 2, "{actual} 的第 {i} 个通道是 {got}，期望接近 {want}");
        }
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
            assert!(
                resolve_in_library(dir, bad).is_err(),
                "{bad} 不该通过校验"
            );
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
        assert!(aurora.set_background(Some("不存在.jpg".to_owned())).is_err());
        // 越界文件名同样拒绝，且不该因为「文件确实存在」而放行。
        assert!(aurora.set_background(Some("../config.json".to_owned())).is_err());
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
        let now = aurora.config().appearance.background.as_ref().expect("有背景");
        assert_eq!(now.file, red.file);
        assert_tint_near(&now.tint, [200, 40, 30]);

        // 切换必须重算 tint，否则加载间隙会闪上一张的颜色。
        aurora.set_background(Some(blue.file.clone())).expect("设蓝");
        let now = aurora.config().appearance.background.as_ref().expect("有背景");
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

        aurora.set_background(Some(a.file.clone())).expect("设为当前");
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
        aurora.set_background(Some(imported.file.clone())).expect("设为当前");

        aurora.remove_background(&imported.file).await.expect("删除");
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
        aurora.set_background(Some(keep.file.clone())).expect("设当前");

        aurora.remove_background(&drop.file).await.expect("删另一张");
        assert_eq!(
            aurora.config().appearance.background.as_ref().map(|b| b.file.as_str()),
            Some(keep.file.as_str())
        );
    }

    #[tokio::test]
    async fn read_background_serves_bytes_and_blocks_traversal() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let aurora = aurora_at(tmp.path());
        let src = write_solid(tmp.path(), "y.png", 32, 32, [4, 5, 6]);
        let imported = aurora.import_background(&src).await.expect("导入");

        let bytes = aurora.read_background(&imported.file).await.expect("读字节");
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
