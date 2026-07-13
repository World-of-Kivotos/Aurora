//! 按钮：主按钮（蓝→粉强调渐变实底）、次按钮（描边幽灵态），均内建悬停/按下的视觉反馈；
//! 另提供 [`spring_press`] 便捷封装，供页面用一枚 [`Animated`](crate::anim::Animated) 缩放值给任意
//! 可点元素加「弹簧按压」形变。
//!
//! 说明：iced 的内置 `button` 不持有逐帧动画状态，故「颜色态反馈」由样式闭包按 `Status` 给出；真正的
//! 弹簧按压缩放需要外部动画值驱动，用 [`spring_press`] 包一层 `float` 实现（页面持 `Animated`，按下设
//! 目标 0.96、松开回 1.0，逐帧把 `value()` 传进来）。

use iced::widget::button::{self, Button};
use iced::widget::{Text, float, text};
use iced::{Background, Border, Element, Vector};

use crate::theme::{self, Tokens};

/// 主按钮：蓝→粉强调渐变实底、白字、悬停加亮、按下微暗。`on_press` 为 `None` 时禁用（无渐变、降透明）。
pub fn primary_button<'a, Message: Clone + 'a>(
    label: &'a str,
    tokens: Tokens,
    on_press: Option<Message>,
) -> Button<'a, Message> {
    let content: Text<'a> = text(label).size(theme::TEXT_BODY);
    Button::new(content)
        .padding([theme::SPACE_SM, theme::SPACE_MD])
        .on_press_maybe(on_press)
        .style(move |_theme, status| {
            let alpha = match status {
                button::Status::Hovered => 1.0,
                button::Status::Pressed => 0.85,
                button::Status::Active => 0.92,
                button::Status::Disabled => 0.45,
            };
            button::Style {
                background: Some(Background::Gradient(
                    tokens.accent_linear().scale_alpha(alpha).into(),
                )),
                text_color: tokens.accent_text,
                border: Border {
                    radius: theme::RADIUS_SM.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        })
}

/// 次按钮：透明底 + 强调描边、随主题取前景文字，悬停填入弱高光。用于非主操作。
pub fn secondary_button<'a, Message: Clone + 'a>(
    label: &'a str,
    tokens: Tokens,
    on_press: Option<Message>,
) -> Button<'a, Message> {
    let content: Text<'a> = text(label).size(theme::TEXT_BODY).color(tokens.on_surface);
    Button::new(content)
        .padding([theme::SPACE_SM, theme::SPACE_MD])
        .on_press_maybe(on_press)
        .style(move |_theme, status| {
            let background = match status {
                button::Status::Hovered | button::Status::Pressed => {
                    Some(Background::Color(tokens.hover))
                }
                _ => None,
            };
            button::Style {
                background,
                text_color: tokens.on_surface,
                border: Border {
                    color: tokens.accent_from,
                    width: 1.0,
                    radius: theme::RADIUS_SM.into(),
                },
                ..button::Style::default()
            }
        })
}

/// 弹簧按压封装：把任意元素套一层 `float` 缩放。`scale` 通常来自页面持有的 [`Animated`]
/// （按下设 0.96、松开回 1.0，逐帧传 `value()`）。scale=1.0 时无变换。
pub fn spring_press<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    scale: f32,
) -> Element<'a, Message> {
    float(content)
        .scale(scale)
        .translate(|_content, _viewport| Vector::ZERO)
        .into()
}
