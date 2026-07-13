//! 主题令牌：亮/暗双主题的完整设计令牌 + 与主题无关的尺度刻度（间距/圆角/字号/字体）。
//!
//! 页面与组件只引用令牌，不写死色值/尺寸，方便整套换肤与统一改版。色彩体系：极光背景蓝→粉斜渐变、
//! 卡片毛玻璃面/描边、多级前景文字、提升面（标题栏/导航栏）、交互态高光、蓝→粉强调渐变。尺度刻度
//! 走 4px 栅格。字体固定微软雅黑（Windows 自带 CJK 字形，无需内嵌）。

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
    /// 极光背景渐变起点（浅蓝端）。
    pub bg_from: Color,
    /// 极光背景渐变终点（粉端）。
    pub bg_to: Color,

    /// 卡片毛玻璃面（半透明，叠在背景上）。
    pub surface: Color,
    /// 卡片描边。
    pub surface_border: Color,
    /// 提升面（标题栏/导航栏，比卡片更实一点，界定结构）。
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
    /// 选中态背景（导航项当前页、列表选中项）。
    pub selected: Color,

    /// 强调渐变起点（蓝）。
    pub accent_from: Color,
    /// 强调渐变终点（粉）。
    pub accent_to: Color,
    /// 强调面上的文字/图标色（主按钮文案）。
    pub accent_text: Color,

    /// 阴影颜色（含 alpha，用于卡片投影）。
    pub shadow: Color,
    /// 窗口兜底底色（透明窗口下极少直接可见，仅作保险）。供页面 agent 取用（外壳走透明底）。
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

    /// 极光背景线性渐变（约 45° 斜向，浅蓝→粉）。背景层的画布走几何渐变（两点式），此处提供 widget
    /// 级（角度式）渐变，供页面 agent 在容器背景复用同款极光（外壳当前未直接使用）。
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
        Mode::Light => Tokens {
            bg_from: Color::from_rgb8(0xA9, 0xC7, 0xFF),
            bg_to: Color::from_rgb8(0xFF, 0xC2, 0xE2),
            surface: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.72),
            surface_border: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.90),
            elevated: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.55),
            elevated_border: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.75),
            on_surface: Color::from_rgb8(0x1B, 0x24, 0x40),
            on_surface_muted: Color::from_rgba8(0x1B, 0x24, 0x40, 0.60),
            title_text: Color::from_rgb8(0x23, 0x30, 0x4F),
            icon: Color::from_rgb8(0x33, 0x41, 0x5C),
            hover: Color::from_rgba8(0x33, 0x41, 0x5C, 0.10),
            selected: Color::from_rgba8(0x6F, 0xA8, 0xFF, 0.22),
            accent_from: Color::from_rgb8(0x6F, 0xA8, 0xFF),
            accent_to: Color::from_rgb8(0xFF, 0x8F, 0xC7),
            accent_text: Color::from_rgb8(0xFF, 0xFF, 0xFF),
            shadow: Color::from_rgba8(0x23, 0x30, 0x4F, 0.18),
            window_base: Color::from_rgb8(0xCF, 0xE0, 0xFF),
        },
        Mode::Dark => Tokens {
            bg_from: Color::from_rgb8(0x1B, 0x2B, 0x52),
            bg_to: Color::from_rgb8(0x3E, 0x23, 0x40),
            surface: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.10),
            surface_border: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.22),
            elevated: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.06),
            elevated_border: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.14),
            on_surface: Color::from_rgb8(0xE8, 0xEE, 0xFF),
            on_surface_muted: Color::from_rgba8(0xE8, 0xEE, 0xFF, 0.62),
            title_text: Color::from_rgb8(0xE6, 0xEC, 0xFF),
            icon: Color::from_rgb8(0xC7, 0xD2, 0xEC),
            hover: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.08),
            selected: Color::from_rgba8(0x6F, 0xA8, 0xFF, 0.26),
            accent_from: Color::from_rgb8(0x6F, 0xA8, 0xFF),
            accent_to: Color::from_rgb8(0xFF, 0x8F, 0xC7),
            accent_text: Color::from_rgb8(0xFF, 0xFF, 0xFF),
            shadow: Color::from_rgba8(0x00, 0x00, 0x00, 0.45),
            window_base: Color::from_rgb8(0x16, 0x21, 0x3C),
        },
    }
}
