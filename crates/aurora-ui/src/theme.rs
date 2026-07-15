//! 主题令牌：亮/暗双主题的完整设计令牌 + 与主题无关的尺度刻度（间距/圆角/字号/字体）。
//!
//! 页面与组件只引用令牌，不写死色值/尺寸，方便整套换肤与统一改版。色彩体系（纯白不透明 + 渐变强调）：
//! 亮色以纯白为基底画布、深色以纯深底为基底，均不透明、无毛玻璃/半透明/角落柔光。蓝→粉斜渐变只作强调
//! 色用在主按钮、选中导航项/列表行、进度、焦点高光、关键图标，不铺满背景。卡片与背景同为不透明纯色，
//! 靠柔和阴影 + 极淡描边分层。另有多级前景文字、提升面（标题栏/导航栏）、交互态高光。尺度刻度走 4px
//! 栅格。字体固定微软雅黑（Windows 自带 CJK 字形，无需内嵌）。

use iced::{Color, Font, Theme, gradient::Linear};

/// UI 默认字体族名。iced 默认内嵌字体不含 CJK，必须在应用设置里把 default_font 指到系统字体，
/// 否则简体中文渲染成豆腐块。微软雅黑 Windows 自带，无需打包文件。
pub const FONT_FAMILY: Font = Font::with_name("Microsoft YaHei");

// ---- 间距刻度（4px 栅格，主题无关）----
/// 极小间距（图标与文字之间等紧凑场景）。
pub const SPACE_XS: f32 = 4.0;
/// 小间距（相关元素成组）。
pub const SPACE_SM: f32 = 8.0;
/// 中间距（卡片内边距、控件行距的默认）。
pub const SPACE_MD: f32 = 16.0;
/// 大间距（分区之间）。
pub const SPACE_LG: f32 = 24.0;
/// 特大间距（页面级留白）。
pub const SPACE_XL: f32 = 32.0;

// ---- 圆角刻度 ----
/// 小圆角（按钮、标签）。
pub const RADIUS_SM: f32 = 8.0;
/// 中圆角（卡片默认）。
pub const RADIUS_MD: f32 = 14.0;
/// 大圆角（大面板、对话框）。供页面 agent 取用（外壳当前未直接使用）。
#[allow(dead_code)]
pub const RADIUS_LG: f32 = 20.0;

// ---- 字号刻度 ----
/// 页面主标题。
pub const TEXT_TITLE: f32 = 24.0;
/// 区块/分组标题。
pub const TEXT_HEADING: f32 = 17.0;
/// 正文默认。
pub const TEXT_BODY: f32 = 14.0;
/// 辅助说明/副标题。
pub const TEXT_CAPTION: f32 = 12.0;

/// 强调渐变的方向角（弧度）。iced 渐变 0 指向正上、顺时针增大；3π/4 ≈ 135° 指向右下，配合蓝在
/// 起点、粉在终点即得「左上蓝 → 右下粉」约 45° 斜向。
const ACCENT_ANGLE: f32 = 3.0 * std::f32::consts::FRAC_PI_4;

/// 主题模式。运行时可切换（亮/暗），亦可后续接系统跟随。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    /// 切换到另一模式。
    pub fn toggled(self) -> Self {
        match self {
            Mode::Light => Mode::Dark,
            Mode::Dark => Mode::Light,
        }
    }

    /// 映射到 iced 内置主题，驱动内置控件（slider/text_input 等）的默认底色。
    pub fn iced_theme(self) -> Theme {
        match self {
            Mode::Light => Theme::Light,
            Mode::Dark => Theme::Dark,
        }
    }
}

/// 一套解析后的主题令牌。所有颜色随 [`Mode`] 变化；尺度刻度是模块级常量（不随主题变）。
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    /// 背景基底色（纯白或纯深底，均不透明）。背景层铺满此色，是「纯白不透明」的主视觉。
    pub bg_from: Color,
    /// 背景次级色（仅供 [`aurora_linear`](Tokens::aurora_linear) 助手复用；背景层只铺 bg_from 纯色）。
    pub bg_to: Color,

    /// 卡片面（不透明纯白/深底；靠阴影 + 描边与背景分层，不靠透明）。
    pub surface: Color,
    /// 卡片描边（极淡，与不透明卡片配合作分层）。
    pub surface_border: Color,
    /// 提升面（标题栏/导航栏，不透明纯色以便作覆盖层干净盖住内容）。
    pub elevated: Color,
    /// 提升面描边。
    pub elevated_border: Color,

    /// 主前景文字（卡片/面板上的标题正文）。
    pub on_surface: Color,
    /// 次级前景文字（副标题/说明/占位）。
    pub on_surface_muted: Color,
    /// 标题栏文字。
    pub title_text: Color,
    /// 图标描边色（细线图标默认取色）。
    pub icon: Color,

    /// 交互悬停高光（叠加在面上的弱高亮）。
    pub hover: Color,
    /// 选中态背景（列表选中行，强调淡底）。导航选中胶囊另用 [`nav_selected`](Tokens::nav_selected)。
    pub selected: Color,

    /// 导航栏底色：比纯白内容区偏灰一档（冷浅灰），让白色选中胶囊 + 投影从中「浮起」形成对比。
    pub nav_rail: Color,
    /// 导航选中胶囊底色（亮色纯白 / 暗色略提升面），配合 [`shadow`](Tokens::shadow) 从导航底浮起。
    pub nav_selected: Color,

    /// 强调渐变起点（蓝）。
    pub accent_from: Color,
    /// 强调渐变终点（粉）。
    pub accent_to: Color,
    /// 强调面上的文字/图标色（主按钮文案）。
    pub accent_text: Color,

    /// 阴影颜色（含 alpha，用于卡片投影）。
    pub shadow: Color,
    /// 窗口基底底色（与 bg_from 一致，不透明窗口的清屏色）。供页面 agent 取用。
    #[allow(dead_code)]
    pub window_base: Color,
}

impl Tokens {
    /// 蓝→粉强调线性渐变（约 45° 斜向）。用于主按钮、进度、高光、选中焦点。
    pub fn accent_linear(&self) -> Linear {
        Linear::new(ACCENT_ANGLE)
            .add_stop(0.0, self.accent_from)
            .add_stop(1.0, self.accent_to)
    }

    /// 背景基底→次级色的线性渐变（约 45° 斜向）。背景层只铺 bg_from 纯色不用此助手；此处提供 widget 级
    /// （角度式）极淡渐变，供页面 agent 在容器背景复用同款极淡底纹（外壳当前未直接使用）。
    #[allow(dead_code)]
    pub fn aurora_linear(&self) -> Linear {
        Linear::new(ACCENT_ANGLE)
            .add_stop(0.0, self.bg_from)
            .add_stop(1.0, self.bg_to)
    }
}

/// 按模式解析令牌。
pub fn tokens(mode: Mode) -> Tokens {
    match mode {
        // 纯白不透明：基底纯白铺满，无角落柔光；卡片不透明纯白 + 极淡描边 + 柔和阴影，靠阴影与描边同白底
        // 分层、界限清晰有浮起感；提升面同为不透明纯白以便作导航覆盖层干净盖住内容；文字深色保证白底可读；
        // 蓝→粉只在强调元素出现。
        Mode::Light => Tokens {
            bg_from: Color::from_rgb8(0xFF, 0xFF, 0xFF),
            bg_to: Color::from_rgb8(0xF5, 0xF7, 0xFC),
            surface: Color::from_rgb8(0xFF, 0xFF, 0xFF),
            surface_border: Color::from_rgba8(0x2A, 0x3A, 0x66, 0.10),
            elevated: Color::from_rgb8(0xFF, 0xFF, 0xFF),
            elevated_border: Color::from_rgba8(0x2A, 0x3A, 0x66, 0.08),
            on_surface: Color::from_rgb8(0x1B, 0x24, 0x36),
            on_surface_muted: Color::from_rgba8(0x1B, 0x24, 0x36, 0.58),
            title_text: Color::from_rgb8(0x23, 0x30, 0x4F),
            icon: Color::from_rgb8(0x3A, 0x47, 0x63),
            hover: Color::from_rgba8(0x2A, 0x3A, 0x66, 0.07),
            selected: Color::from_rgba8(0x6F, 0xA8, 0xFF, 0.20),
            nav_rail: Color::from_rgb8(0xF3, 0xF5, 0xF9),
            nav_selected: Color::from_rgb8(0xFF, 0xFF, 0xFF),
            accent_from: Color::from_rgb8(0x6F, 0xA8, 0xFF),
            accent_to: Color::from_rgb8(0xFF, 0x8F, 0xC7),
            accent_text: Color::from_rgb8(0xFF, 0xFF, 0xFF),
            shadow: Color::from_rgba8(0x23, 0x30, 0x4F, 0.14),
            window_base: Color::from_rgb8(0xFF, 0xFF, 0xFF),
        },
        // 纯深底不透明：中性深炭底铺满，无角落柔光；卡片与提升面同为不透明深底面板（#181D2B），比背景略亮
        // 一档、靠阴影与描边分层，提升面借此干净作导航覆盖层；蓝→粉同样只在强调元素出现，与亮色一致的
        // 「纯底 + 渐变强调」而非彩色铺满。
        Mode::Dark => Tokens {
            bg_from: Color::from_rgb8(0x11, 0x14, 0x20),
            bg_to: Color::from_rgb8(0x1A, 0x1F, 0x2E),
            surface: Color::from_rgb8(0x18, 0x1D, 0x2B),
            surface_border: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.12),
            elevated: Color::from_rgb8(0x18, 0x1D, 0x2B),
            elevated_border: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.10),
            on_surface: Color::from_rgb8(0xE8, 0xEE, 0xFF),
            on_surface_muted: Color::from_rgba8(0xE8, 0xEE, 0xFF, 0.62),
            title_text: Color::from_rgb8(0xE6, 0xEC, 0xFF),
            icon: Color::from_rgb8(0xC7, 0xD2, 0xEC),
            hover: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.07),
            selected: Color::from_rgba8(0x6F, 0xA8, 0xFF, 0.28),
            nav_rail: Color::from_rgb8(0x1D, 0x23, 0x31),
            nav_selected: Color::from_rgb8(0x2A, 0x31, 0x43),
            accent_from: Color::from_rgb8(0x6F, 0xA8, 0xFF),
            accent_to: Color::from_rgb8(0xFF, 0x8F, 0xC7),
            accent_text: Color::from_rgb8(0xFF, 0xFF, 0xFF),
            shadow: Color::from_rgba8(0x00, 0x00, 0x00, 0.50),
            window_base: Color::from_rgb8(0x11, 0x14, 0x20),
        },
    }
}
