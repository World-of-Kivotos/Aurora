//! 输入类组件：带标签与数值回显的滑条封装。
//!
//! 封装 iced 内置 `slider`，统一「左标签 + 中滑条 + 右数值」的三段版式，页面调设置项时不必重复排布。

use std::ops::RangeInclusive;

use iced::widget::{row, slider, text};
use iced::{Alignment, Element, Length};

use crate::theme::{self, Tokens};

/// 带标签滑条：左固定宽标签、中自适应滑条、右固定宽数值回显。
///
/// - `on_change`：滑动回调，产出页面自有 Message。
/// - `value_text`：右侧数值展示文本（由页面按需格式化，如 `"4096 MB"`）。
pub fn labeled_slider<'a, Message, F>(
    label: &'a str,
    range: RangeInclusive<f32>,
    value: f32,
    step: f32,
    on_change: F,
    value_text: String,
    tokens: Tokens,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
    F: Fn(f32) -> Message + 'a,
{
    row![
        text(label)
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface)
            .width(Length::Fixed(180.0)),
        slider(range, value, on_change).step(step),
        text(value_text)
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted)
            .width(Length::Fixed(72.0)),
    ]
    .spacing(theme::SPACE_MD)
    .align_y(Alignment::Center)
    .into()
}
