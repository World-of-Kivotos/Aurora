//! 左侧导航项：图标 + （展开时）文字，带选中态与悬停高光。
//!
//! 展开是导航栏整体宽度的弹簧动画（由 app 持有并驱动），单个导航项据传入的 `reveal`(0..1) 决定是否
//! 渲染文字并给文字上淡入透明度：收起态仅图标居中，展开态图标 + 文字左对齐。宽度收敛由 app 侧的
//! `Animated` 负责，导航项本身无状态。

use iced::widget::{button, container, row, text};
use iced::{Background, Border, Element, Length, Shadow, Vector};

use crate::theme::{self, Tokens};
use crate::widgets::icon::{Icon, icon};

/// 一枚导航项。
///
/// - `glyph`/`label`：图标与文案。
/// - `selected`：是否为当前页（渲染纯白圆角胶囊 + 柔和投影 + 强调色图标，从偏灰导航底浮起）。
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
    // 选中用强调色描边呼应主题；未选中收敛到次级前景灰，让选中胶囊里的强调图标更跳。
    let icon_color = if selected {
        tokens.accent_from
    } else {
        tokens.on_surface_muted
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
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            // 选中=白胶囊 + 柔和投影浮起；未选中 hover=极淡中性高光（同款圆角暗示可点，无投影）；
            // 其余=透明贴底。投影仅在选中态给非零 alpha，button 据此才绘制阴影 quad。
            let (background, shadow) = if selected {
                (
                    Some(Background::Color(tokens.nav_selected)),
                    Shadow {
                        color: tokens.shadow,
                        offset: Vector::new(0.0, 2.0),
                        blur_radius: 10.0,
                    },
                )
            } else if hovered {
                (Some(Background::Color(tokens.hover)), Shadow::default())
            } else {
                (None, Shadow::default())
            };
            button::Style {
                background,
                text_color: tokens.on_surface,
                border: Border {
                    radius: theme::RADIUS_MD.into(),
                    ..Border::default()
                },
                shadow,
                ..button::Style::default()
            }
        })
        .into()
}
