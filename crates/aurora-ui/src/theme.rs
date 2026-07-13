//! 最小亮/暗主题令牌。
//!
//! 只暴露本验收切片需要的令牌：极光背景渐变端点、卡片面/描边、前景文字、
//! 蓝→粉强调渐变。页面与画布只引用令牌，不写死色值，方便后续整套换肤。
//! UI 文本一律用拉丁字符：iced 默认内嵌字体不含 CJK 字形，未接入系统字体前
//! 中文会渲染成豆腐块，故此切片界面文案暂用英文。

use iced::{Color, Theme};

/// 主题模式。运行时可切换以验证令牌在两套配色下都成立。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    pub fn toggled(self) -> Self {
        match self {
            Mode::Light => Mode::Dark,
            Mode::Dark => Mode::Light,
        }
    }

    /// 映射到 iced 内置主题，驱动内置控件（按钮等）的默认配色。
    pub fn iced_theme(self) -> Theme {
        match self {
            Mode::Light => Theme::Light,
            Mode::Dark => Theme::Dark,
        }
    }
}

/// 一套解析后的主题令牌。
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    /// 极光背景渐变起点（浅蓝端）。
    pub bg_from: Color,
    /// 极光背景渐变终点（粉端）。
    pub bg_to: Color,
    /// 卡片面（半透明，叠在渐变上形成毛玻璃观感）。
    pub surface: Color,
    /// 卡片描边。
    pub surface_border: Color,
    /// 卡片上文字。
    pub on_surface: Color,
    /// 卡片副标题/弱化文字。
    pub on_surface_muted: Color,
    /// 标题栏文字。
    pub title_text: Color,
    /// 标题栏图标描边色。
    pub icon: Color,
    /// 强调渐变起点（蓝）。
    pub accent_from: Color,
    /// 强调渐变终点（粉）。
    pub accent_to: Color,
    /// 窗口兜底底色（画布未覆盖到的极小区域）。
    pub window_base: Color,
}

/// 按模式解析令牌。
pub fn tokens(mode: Mode) -> Tokens {
    match mode {
        Mode::Light => Tokens {
            bg_from: Color::from_rgb8(0xA9, 0xC7, 0xFF),
            bg_to: Color::from_rgb8(0xFF, 0xC2, 0xE2),
            surface: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.72),
            surface_border: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.90),
            on_surface: Color::from_rgb8(0x1B, 0x24, 0x40),
            on_surface_muted: Color::from_rgba8(0x1B, 0x24, 0x40, 0.60),
            title_text: Color::from_rgb8(0x23, 0x30, 0x4F),
            icon: Color::from_rgb8(0x33, 0x41, 0x5C),
            accent_from: Color::from_rgb8(0x6F, 0xA8, 0xFF),
            accent_to: Color::from_rgb8(0xFF, 0x8F, 0xC7),
            window_base: Color::from_rgb8(0xCF, 0xE0, 0xFF),
        },
        Mode::Dark => Tokens {
            bg_from: Color::from_rgb8(0x1B, 0x2B, 0x52),
            bg_to: Color::from_rgb8(0x3E, 0x23, 0x40),
            surface: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.10),
            surface_border: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0.22),
            on_surface: Color::from_rgb8(0xE8, 0xEE, 0xFF),
            on_surface_muted: Color::from_rgba8(0xE8, 0xEE, 0xFF, 0.62),
            title_text: Color::from_rgb8(0xE6, 0xEC, 0xFF),
            icon: Color::from_rgb8(0xC7, 0xD2, 0xEC),
            accent_from: Color::from_rgb8(0x6F, 0xA8, 0xFF),
            accent_to: Color::from_rgb8(0xFF, 0x8F, 0xC7),
            window_base: Color::from_rgb8(0x16, 0x21, 0x3C),
        },
    }
}
