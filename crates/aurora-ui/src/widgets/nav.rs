//! 左侧导航项：图标 + （展开时）文字，带选中态与悬停高光。
//!
//! 展开是导航栏整体宽度的弹簧动画（由 app 持有并驱动），单个导航项据传入的 `reveal`(0..1) 决定是否
//! 渲染文字并给文字上淡入透明度：收起态仅图标居中，展开态图标 + 文字左对齐。宽度收敛由 app 侧的
//! `Animated` 负责，导航项本身无状态。

use iced::widget::{button, container, row, text};
use iced::{Background, Border, Element, Length};

use crate::theme::{self, Tokens};
use crate::widgets::icon::{Icon, icon};

/// 一枚导航项。
///
/// - `glyph`/`label`：图标与文案。
/// - `selected`：是否为当前页（给强调选中底 + 强调色图标）。
/// - `reveal`：展开进度 0..1；<0.5 视作收起（仅图标），否则渲染文字并按 reveal 给淡入。
/// - `on_press`：点击消息（通常是 `Message::Navigate(screen)`）。
pub fn nav_item<'a, Message: Clone + 'a>(
    glyph: Icon,
    label: &'a str,
    selected: bool,
    reveal: f32,
    tokens: Tokens,
    on_press: Message,
) -> Element<'a, Message> {
    let icon_color = if selected {
        tokens.accent_from
    } else {
        tokens.icon
    };
    let glyph_el = icon(glyph, 20.0, icon_color);

    let content: Element<'a, Message> = if reveal > 0.5 {
        let text_color = if selected {
            tokens.on_surface
        } else {
            tokens.on_surface_muted
        };
        row![
            glyph_el,
            text(label)
                .size(theme::TEXT_BODY)
                .color(text_color.scale_alpha(reveal)),
        ]
        .spacing(theme::SPACE_SM)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        // 收起态：图标居中占满按钮宽度。
        container(glyph_el).center_x(Length::Fill).into()
    };

    button(content)
        .width(Length::Fill)
        .padding([theme::SPACE_SM, theme::SPACE_SM + 2.0])
        .on_press(on_press)
        .style(move |_theme, status| {
            let background = if selected {
                Some(Background::Color(tokens.selected))
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                Some(Background::Color(tokens.hover))
            } else {
                None
            };
            button::Style {
                background,
                text_color: tokens.on_surface,
                border: Border {
                    radius: theme::RADIUS_SM.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        })
        .into()
}
