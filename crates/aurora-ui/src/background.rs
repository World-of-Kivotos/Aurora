//! 应用级背景层（纯白不透明）：用主题基底色（Light 纯白 / Dark 纯深底）铺满整窗，作为「纯白不透明」
//! 视觉的底。彩色强调渐变留给按钮/选中项等交互元素，背景只铺纯色、不铺渐变、无半透明、无角落柔光。
//!
//! 纯色底不随时间变化，故本层无动画状态、不驱动帧订阅。

use iced::mouse;
use iced::widget::canvas::{self, Frame, Path as CanvasPath};
use iced::widget::canvas as canvas_fn;
use iced::{Color, Element, Fill, Point, Rectangle};

use crate::theme::Tokens;

/// 背景层：无状态的纯色底。
#[derive(Debug, Clone, Default)]
pub struct Background;

impl Background {
    /// 默认背景（纯色，无动画、无自定义图）。
    pub fn new() -> Self {
        Self
    }

    /// 背景元素：用主题基底色铺满整窗。
    pub fn view<'a, Message: 'a>(&self, tokens: Tokens) -> Element<'a, Message> {
        canvas_fn(SolidCanvas {
            base: tokens.bg_from,
        })
        .width(Fill)
        .height(Fill)
        .into()
    }
}

/// 纯色背景画布程序：用基底色铺满整帧。
struct SolidCanvas {
    base: Color,
}

impl<Message> canvas::Program<Message> for SolidCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        frame.fill(&CanvasPath::rectangle(Point::ORIGIN, size), self.base);
        vec![frame.into_geometry()]
    }
}
