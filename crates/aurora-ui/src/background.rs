//! 应用级背景层（白底为主 + 渐变强调）：以主题基底近白/深底铺满，仅在两个对角叠极淡的蓝/粉柔光作
//! 点缀，可选缓慢流动，并预留「用户换图」接口位。彩色强调渐变留给按钮/选中项等交互元素，背景不铺满。
//!
//! 这是亚克力失效时的降级兜底，也是无图时的默认底。当 window-vibrancy 亚克力生效时，app 会传
//! `draw_gradient=false` 让本层透明，好让亚克力/桌面透出；亚克力失效或未定时画基底 + 柔光。
//!
//! 流动默认关闭：持续流动意味着帧订阅永不收敛（与「收敛即停省电」冲突），故作为显式开关，开启后
//! [`animating`](Background::animating) 返回真、app 才为它挂帧。

use std::path::{Path, PathBuf};

use iced::mouse;
use iced::widget::canvas::gradient::Linear;
use iced::widget::canvas::{self, Frame, Path as CanvasPath};
use iced::widget::{canvas as canvas_fn, container, text};
use iced::{Color, Element, Fill, Point, Rectangle, Size};

use crate::theme::Tokens;

/// 背景层状态：流动相位、是否流动、用户自定义背景图路径（换图接口位）。
#[derive(Debug, Clone, Default)]
pub struct Background {
    /// 流动相位（弧度累加），驱动渐变缓慢位移。
    phase: f32,
    /// 是否启用缓慢流动。默认关闭以省电。
    flowing: bool,
    /// 用户自定义背景图路径。设置后此处为换图接口位；实际位图渲染需启用 iced `image` 特性并在本层
    /// 叠一枚 `image` 控件，属后续接入项，当前仍以渐变兜底。设置页 agent 接入后经 [`set_image`]/
    /// [`image_path`] 读写此位。
    #[allow(dead_code)]
    image: Option<PathBuf>,
}

impl Background {
    /// 默认背景（静态渐变，不流动，无自定义图）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 推进流动相位（仅在流动开启时累加；夹到 2π 周期内防止浮点无限增长）。
    pub fn step(&mut self, dt: f32) {
        if self.flowing {
            self.phase = (self.phase + dt * FLOW_SPEED) % std::f32::consts::TAU;
        }
    }

    /// 是否需要持续帧订阅（仅流动时）。
    pub fn animating(&self) -> bool {
        self.flowing
    }

    /// 开关缓慢流动。供设置页 agent 接入（外壳默认静态）。
    #[allow(dead_code)]
    pub fn set_flowing(&mut self, flowing: bool) {
        self.flowing = flowing;
    }

    /// 设置/清除用户自定义背景图（换图接口位）。供设置页 agent 接入。
    #[allow(dead_code)]
    pub fn set_image(&mut self, image: Option<PathBuf>) {
        self.image = image;
    }

    /// 当前自定义背景图路径（供设置页回显）。供设置页 agent 接入。
    #[allow(dead_code)]
    pub fn image_path(&self) -> Option<&Path> {
        self.image.as_deref()
    }

    /// 背景元素。`draw_gradient=false`（亚克力生效）时返回透明占位，让亚克力透出；否则画极光渐变。
    pub fn view<'a, Message: 'a>(
        &self,
        tokens: Tokens,
        draw_gradient: bool,
    ) -> Element<'a, Message> {
        if !draw_gradient {
            // 亚克力生效：本层透明，让亚克力/桌面透出。
            return container(text("")).width(Fill).height(Fill).into();
        }
        let program = AuroraCanvas {
            base: tokens.bg_from,
            glow_a: tokens.accent_from,
            glow_b: tokens.accent_to,
            phase: self.phase,
        };
        canvas_fn(program).width(Fill).height(Fill).into()
    }
}

/// 流动角速度（弧度/秒）。取小值以「缓慢」，避免喧宾夺主。
const FLOW_SPEED: f32 = 0.25;

/// 角落柔光的起点 alpha（终点透明）。取极低值：背景只是白/深底加一抹光，主视觉仍是基底、不弄脏。
const GLOW_ALPHA: f32 = 0.10;

/// 背景画布程序：基底铺满 + 两个对角的极淡强调色柔光。持基底色、两枚柔光色与流动相位，流动开启时按
/// 相位轻移柔光锚点形成缓慢漂动。
struct AuroraCanvas {
    base: Color,
    glow_a: Color,
    glow_b: Color,
    phase: f32,
}

impl<Message> canvas::Program<Message> for AuroraCanvas {
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

        // 基底铺满：白底为主的主视觉即此层。
        frame.fill(&CanvasPath::rectangle(Point::ORIGIN, size), self.base);

        // 左上蓝、右下粉两抹极淡柔光。相位在流动开启时轻移锚点（相位为 0 即固定对角），alpha 极低故只
        // 在角落隐约透出、不铺满、不弄脏基底。
        let sway = self.phase.sin() * 0.06;
        corner_glow(
            &mut frame,
            size,
            Point::new(size.width * sway.max(0.0), size.height * (-sway).max(0.0)),
            Point::new(size.width * 0.6, size.height * 0.6),
            self.glow_a,
        );
        corner_glow(
            &mut frame,
            size,
            Point::new(
                size.width * (1.0 - (-sway).max(0.0)),
                size.height * (1.0 - sway.max(0.0)),
            ),
            Point::new(size.width * 0.4, size.height * 0.4),
            self.glow_b,
        );

        vec![frame.into_geometry()]
    }
}

/// 一抹角落柔光：从 `corner` 到 `fade_to` 的线性渐变（起点为强调色的极淡 alpha、终点透明），铺满整帧后
/// 即形成「从该角渐隐」的柔光。
fn corner_glow(frame: &mut Frame, size: Size, corner: Point, fade_to: Point, tint: Color) {
    let gradient = Linear::new(corner, fade_to)
        .add_stop(0.0, tint.scale_alpha(GLOW_ALPHA))
        .add_stop(1.0, Color::TRANSPARENT);
    frame.fill(&CanvasPath::rectangle(Point::ORIGIN, size), gradient);
}
