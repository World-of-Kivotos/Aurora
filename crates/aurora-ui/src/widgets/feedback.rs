//! 加载/进度/占位类反馈组件：确定进度条、加载提示。
//!
//! 确定进度条封装 iced 内置 `progress_bar` 并套强调色。加载提示是静态的强调色文案——不定量的旋转
//! 指示需要逐帧动画状态，页面若要旋转指示可自持一枚 [`Animated`](crate::anim::Animated) 驱动角度，
//! 此处只给不依赖动画状态的静态版本，保证组件库无隐藏帧订阅需求。

use iced::widget::{progress_bar, text};
use iced::{Background, Border, Element};

use crate::theme::{self, Tokens};

/// 确定进度条：`fraction` 取 0..1，超出自动钳制。用强调色作进度条填充。
pub fn progress<'a, Message: 'a>(fraction: f32, tokens: Tokens) -> Element<'a, Message> {
    let value = fraction.clamp(0.0, 1.0);
    progress_bar(0.0..=1.0, value)
        .style(move |_theme| progress_bar::Style {
            background: Background::Color(tokens.surface),
            bar: Background::Gradient(tokens.accent_linear().into()),
            border: Border {
                radius: theme::RADIUS_SM.into(),
                ..Border::default()
            },
        })
        .into()
}

/// 加载提示：一行强调色文案（如「载入中…」）。静态，不引入帧订阅。
pub fn loading<'a, Message: 'a>(label: &'a str, tokens: Tokens) -> Element<'a, Message> {
    text(label)
        .size(theme::TEXT_BODY)
        .color(tokens.accent_from)
        .into()
}
