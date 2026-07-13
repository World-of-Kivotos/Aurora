//! 细线描边图标（Lucide 式基调），全部用画布描边自绘：不引入字体/表情符号依赖，描边色跟随主题。
//!
//! 每个图标在 [0,1]×[0,1] 归一坐标里以线段/圆/矩形绘制，再按传入尺寸缩放。新增图标只需在 [`Icon`]
//! 加一枚变体并在 [`draw_glyph`] 补一段绘制。

use iced::mouse;
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::widget::{Canvas, canvas as canvas_fn};
use iced::{Color, Element, Point, Rectangle, Size};

/// 描边图标种类。部分变体（菜单/播放/加号/文件夹）供页面 agent 取用，外壳暂未全部用到。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Icon {
    /// 主页（房子）。
    Home,
    /// 账户（人像）。
    Account,
    /// 版本（层叠）。
    Versions,
    /// Mod（包裹盒）。
    Mods,
    /// 设置（滑杆）。
    Settings,
    /// 最小化（横线）。
    Minimize,
    /// 关闭（叉）。
    Close,
    /// 菜单/展开（三横线）。
    Menu,
    /// 搜索（放大镜）。
    Search,
    /// 播放/启动（三角）。
    Play,
    /// 新增（加号）。
    Plus,
    /// 文件夹（带页签的矩形）。
    Folder,
}

/// 用描边自绘的图标画布程序。借用颜色，按控件 bounds 缩放绘制。
pub struct LineIcon {
    glyph: Icon,
    color: Color,
    /// 描边宽度（像素）。随图标尺寸给定，保证细线观感在不同尺寸下一致。
    stroke_width: f32,
}

/// 构造一枚图标元素：`size` 为边长（像素），`color` 为描边色。描边宽度按尺寸取约 1/12，最细 1.2px。
pub fn icon<'a, Message: 'a>(glyph: Icon, size: f32, color: Color) -> Element<'a, Message> {
    let program = LineIcon {
        glyph,
        color,
        stroke_width: (size / 12.0).max(1.2),
    };
    let canvas: Canvas<LineIcon, Message> = canvas_fn(program).width(size).height(size);
    canvas.into()
}

impl<Message> canvas::Program<Message> for LineIcon {
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
        let stroke = Stroke::default()
            .with_width(self.stroke_width)
            .with_color(self.color)
            .with_line_cap(canvas::LineCap::Round)
            .with_line_join(canvas::LineJoin::Round);
        draw_glyph(&mut frame, self.glyph, size, stroke);
        vec![frame.into_geometry()]
    }
}

/// 归一坐标到画布像素：`p(0.5, 0.5)` 即中心。
fn draw_glyph(frame: &mut Frame, glyph: Icon, size: Size, stroke: Stroke) {
    let p = |x: f32, y: f32| Point::new(x * size.width, y * size.height);
    let line = |frame: &mut Frame, a: (f32, f32), b: (f32, f32)| {
        frame.stroke(&Path::line(p(a.0, a.1), p(b.0, b.1)), stroke);
    };

    match glyph {
        Icon::Home => {
            // 屋顶（人字）+ 屋身（左、下、右三边，顶开口）。
            line(frame, (0.16, 0.50), (0.50, 0.20));
            line(frame, (0.50, 0.20), (0.84, 0.50));
            line(frame, (0.24, 0.46), (0.24, 0.82));
            line(frame, (0.24, 0.82), (0.76, 0.82));
            line(frame, (0.76, 0.82), (0.76, 0.46));
        }
        Icon::Account => {
            // 头（圆）+ 肩（两斜边合一底边，敞口成人形）。
            frame.stroke(&Path::circle(p(0.5, 0.34), size.width * 0.16), stroke);
            line(frame, (0.22, 0.82), (0.34, 0.58));
            line(frame, (0.66, 0.58), (0.78, 0.82));
            line(frame, (0.22, 0.82), (0.78, 0.82));
        }
        Icon::Versions => {
            // 层叠：上菱形 + 下方一道 V，表达多层版本。
            line(frame, (0.50, 0.18), (0.84, 0.38));
            line(frame, (0.84, 0.38), (0.50, 0.58));
            line(frame, (0.50, 0.58), (0.16, 0.38));
            line(frame, (0.16, 0.38), (0.50, 0.18));
            line(frame, (0.16, 0.56), (0.50, 0.76));
            line(frame, (0.50, 0.76), (0.84, 0.56));
        }
        Icon::Mods => {
            // 包裹盒：外框 + 顶盖分割线 + 中缝。
            frame.stroke(
                &Path::rounded_rectangle(
                    p(0.18, 0.24),
                    Size::new(size.width * 0.64, size.height * 0.56),
                    iced::border::Radius::from(size.width * 0.06),
                ),
                stroke,
            );
            line(frame, (0.18, 0.42), (0.82, 0.42));
            line(frame, (0.50, 0.42), (0.50, 0.80));
        }
        Icon::Settings => {
            // 三道滑杆，每道一枚旋钮，错落分布。
            for (i, knob_x) in [(0usize, 0.66f32), (1, 0.36), (2, 0.60)] {
                let y = 0.30 + i as f32 * 0.20;
                line(frame, (0.18, y), (0.82, y));
                frame.stroke(
                    &Path::circle(p(knob_x, y), size.width * 0.07),
                    stroke,
                );
            }
        }
        Icon::Minimize => line(frame, (0.22, 0.50), (0.78, 0.50)),
        Icon::Close => {
            line(frame, (0.26, 0.26), (0.74, 0.74));
            line(frame, (0.74, 0.26), (0.26, 0.74));
        }
        Icon::Menu => {
            line(frame, (0.20, 0.32), (0.80, 0.32));
            line(frame, (0.20, 0.50), (0.80, 0.50));
            line(frame, (0.20, 0.68), (0.80, 0.68));
        }
        Icon::Search => {
            frame.stroke(&Path::circle(p(0.44, 0.44), size.width * 0.24), stroke);
            line(frame, (0.62, 0.62), (0.82, 0.82));
        }
        Icon::Play => {
            // 实心感三角（描边闭合）。
            line(frame, (0.34, 0.24), (0.78, 0.50));
            line(frame, (0.78, 0.50), (0.34, 0.76));
            line(frame, (0.34, 0.76), (0.34, 0.24));
        }
        Icon::Plus => {
            line(frame, (0.50, 0.22), (0.50, 0.78));
            line(frame, (0.22, 0.50), (0.78, 0.50));
        }
        Icon::Folder => {
            // 页签 + 主体。
            line(frame, (0.18, 0.32), (0.44, 0.32));
            line(frame, (0.44, 0.32), (0.52, 0.42));
            frame.stroke(
                &Path::rounded_rectangle(
                    p(0.18, 0.32),
                    Size::new(size.width * 0.64, size.height * 0.42),
                    iced::border::Radius::from(size.width * 0.05),
                ),
                stroke,
            );
        }
    }
}
