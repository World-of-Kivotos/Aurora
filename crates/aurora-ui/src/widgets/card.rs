//! 卡片与版式组件：毛玻璃卡片、可滑动卡片、区块标题、页面头、空态占位。
//!
//! 全部只借用 [`Tokens`]（Copy）着色，返回 `Element`，供页面直接嵌入。可滑动卡片用 iced 的 `float`
//! 做位移变换（不触发重排，适合列表滑入/共享元素过渡的位移插值）。

use iced::widget::{Container, column, container, float, text};
use iced::{Border, Element, Vector};

use crate::theme::{self, Tokens};
use crate::widgets::icon::{Icon, icon};

/// 毛玻璃容器样式闭包。用于给自定义 `container` 套玻璃面/描边/阴影，页面需要非默认内边距时用它。
pub fn glass_style(tokens: Tokens) -> impl Fn(&iced::Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(iced::Background::Color(tokens.surface)),
        border: Border {
            color: tokens.surface_border,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        shadow: iced::Shadow {
            color: tokens.shadow,
            offset: Vector::new(0.0, 6.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

/// 毛玻璃卡片：默认 [`SPACE_MD`](theme::SPACE_MD) 内边距的玻璃面容器，直接包住内容即用。
pub fn glass_card<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    tokens: Tokens,
) -> Container<'a, Message> {
    container(content)
        .padding(theme::SPACE_MD)
        .style(glass_style(tokens))
}

/// 可滑动卡片：在毛玻璃卡片外套一层 `float` 位移。`offset` 为 (x, y) 像素位移（通常由弹簧
/// [`Animated`](crate::anim::Animated) 驱动、收敛到 0），用于列表滑入/滑出与重排的位移插值。
/// 位移只影响绘制、不改变布局占位，故不会把周围元素挤动。
pub fn sliding_card<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    offset: (f32, f32),
    tokens: Tokens,
) -> Element<'a, Message> {
    float(glass_card(content, tokens))
        .translate(move |_content, _viewport| Vector::new(offset.0, offset.1))
        .into()
}

/// 区块/分组标题（用于卡片内或页面内的次级分节）。
pub fn section_title<'a, Message: 'a>(title: &'a str, tokens: Tokens) -> Element<'a, Message> {
    text(title)
        .size(theme::TEXT_HEADING)
        .color(tokens.on_surface)
        .into()
}

/// 页面头：主标题 + 一句说明。各页顶部统一用它对齐版式。
pub fn page_header<'a, Message: 'a>(
    title: &'a str,
    subtitle: &'a str,
    tokens: Tokens,
) -> Element<'a, Message> {
    column![
        text(title).size(theme::TEXT_TITLE).color(tokens.on_surface),
        text(subtitle)
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted),
    ]
    .spacing(theme::SPACE_XS)
    .into()
}

/// 空态占位：居中的图标 + 标题 + 提示。用于列表为空、未登录、无搜索结果等场景。
pub fn empty_state<'a, Message: 'a>(
    glyph: Icon,
    title: &'a str,
    hint: &'a str,
    tokens: Tokens,
) -> Element<'a, Message> {
    let body = column![
        icon(glyph, 48.0, tokens.on_surface_muted),
        text(title).size(theme::TEXT_HEADING).color(tokens.on_surface),
        text(hint)
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted),
    ]
    .spacing(theme::SPACE_SM)
    .align_x(iced::Alignment::Center);

    iced::widget::center(body).into()
}
