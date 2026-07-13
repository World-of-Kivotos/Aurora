//! 演示画布：极光渐变背景 + 三张弹簧卡片 + 命中测试；以及标题栏用的细线图标。
//!
//! 之所以用单个 [`Canvas`] 承载背景与卡片，而非用容器摆放子控件：iced 没有便捷的
//! 绝对定位，而弹簧演示需要按帧把卡片放到任意像素位置并做像素级命中测试，画布是
//! 最直接可控的方式。卡片状态（弹簧）由上层 App 持有，本画布只借用只读引用绘制，
//! 点击时通过 [`canvas::Action::publish`] 把 [`Message::CardClicked`] 上抛给 App。

use iced::mouse;
use iced::widget::canvas::{self, Frame, Path, Stroke, Text};
use iced::widget::canvas::gradient::Linear;
use iced::{Color, Point, Rectangle, Size};

use crate::Message;
use crate::anim::{Params, Spring};
use crate::theme::Tokens;

/// 卡片被点击时抬起的像素距离（向上为负）。
pub const LIFT_DISTANCE: f32 = 96.0;

// 布局常量（画布本地坐标，原点为画布左上角）。
/// 顶部标题栏预留高度。画布据此下压卡片基线，App 的标题栏覆盖层用同一值定高。
pub const TITLE_H: f32 = 48.0;
const CARD_W: f32 = 136.0;
const CARD_H: f32 = 196.0;
const CARD_GAP: f32 = 18.0;
const CARD_RADIUS: f32 = 16.0;
const CARD_PAD: f32 = 14.0;

/// 一张演示卡片：一枚垂直位移弹簧 + 抬起状态。不同卡片用不同预设以对比手感。
pub struct Card {
    pub title: &'static str,
    pub subtitle: &'static str,
    pub lift: Spring,
    pub lifted: bool,
}

impl Card {
    pub fn new(title: &'static str, subtitle: &'static str, params: Params) -> Self {
        Self {
            title,
            subtitle,
            lift: Spring::new(0.0, params),
            lifted: false,
        }
    }

    /// 切换抬起/落回。只改弹簧目标，保留速度：连点可见「打断 + 惯性」。
    pub fn toggle(&mut self) {
        self.lifted = !self.lifted;
        self.lift
            .set_target(if self.lifted { -LIFT_DISTANCE } else { 0.0 });
    }
}

fn card_rect(index: usize, count: usize, size: Size, lift_current: f32) -> Rectangle {
    let total = CARD_W * count as f32 + CARD_GAP * (count.saturating_sub(1)) as f32;
    let start_x = (size.width - total) / 2.0;
    let x = start_x + index as f32 * (CARD_W + CARD_GAP);
    let baseline_y = TITLE_H + (size.height - TITLE_H - CARD_H) / 2.0;
    Rectangle {
        x,
        y: baseline_y + lift_current,
        width: CARD_W,
        height: CARD_H,
    }
}

/// 演示场景画布程序。借用 App 的卡片切片与当前令牌。
pub struct DemoScene<'a> {
    pub cards: &'a [Card],
    pub tokens: Tokens,
}

impl DemoScene<'_> {
    fn hit(&self, bounds: Rectangle, cursor: mouse::Cursor) -> Option<usize> {
        let pos = cursor.position_in(bounds)?;
        let size = bounds.size();
        self.cards.iter().enumerate().find_map(|(i, card)| {
            card_rect(i, self.cards.len(), size, card.lift.current)
                .contains(pos)
                .then_some(i)
        })
    }
}

impl canvas::Program<Message> for DemoScene<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
            && let Some(index) = self.hit(bounds, cursor)
        {
            return Some(canvas::Action::publish(Message::CardClicked(index)).and_capture());
        }
        None
    }

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

        // 极光背景：浅蓝→粉 斜向线性渐变，从左上角到右下角，填满整个画布。几何层渐变用
        // 绝对起止点表达方向，对角线即约 45° 斜向。
        let bg = Linear::new(Point::ORIGIN, Point::new(size.width, size.height))
            .add_stop(0.0, self.tokens.bg_from)
            .add_stop(1.0, self.tokens.bg_to);
        frame.fill(&Path::rectangle(Point::ORIGIN, size), bg);

        let count = self.cards.len();
        for (i, card) in self.cards.iter().enumerate() {
            let r = card_rect(i, count, size, card.lift.current);
            let card_path = Path::rounded_rectangle(
                Point::new(r.x, r.y),
                Size::new(r.width, r.height),
                iced::border::Radius::from(CARD_RADIUS),
            );
            // 半透明面 + 描边：叠在渐变上形成毛玻璃观感。
            frame.fill(&card_path, self.tokens.surface);
            frame.stroke(
                &card_path,
                Stroke::default()
                    .with_width(1.0)
                    .with_color(self.tokens.surface_border),
            );

            // 顶部强调条：蓝→粉横向渐变，点明主题强调色。
            let bar_top_left = Point::new(r.x + CARD_PAD, r.y + CARD_PAD);
            let accent = Linear::new(
                bar_top_left,
                Point::new(r.x + r.width - CARD_PAD, r.y + CARD_PAD),
            )
            .add_stop(0.0, self.tokens.accent_from)
            .add_stop(1.0, self.tokens.accent_to);
            frame.fill(
                &Path::rounded_rectangle(
                    bar_top_left,
                    Size::new(r.width - CARD_PAD * 2.0, 6.0),
                    iced::border::Radius::from(3.0),
                ),
                accent,
            );

            // 文案。fill_text 渲染在所有图层之上，正好当作卡片标签。
            frame.fill_text(Text {
                content: card.title.to_string(),
                position: Point::new(r.x + CARD_PAD, r.y + 40.0),
                color: self.tokens.on_surface,
                size: iced::Pixels(22.0),
                ..Text::default()
            });
            frame.fill_text(Text {
                content: card.subtitle.to_string(),
                position: Point::new(r.x + CARD_PAD, r.y + 74.0),
                color: self.tokens.on_surface_muted,
                size: iced::Pixels(13.0),
                ..Text::default()
            });
            frame.fill_text(Text {
                content: format!("[ {} ]", if card.lifted { "lifted" } else { "resting" }),
                position: Point::new(r.x + CARD_PAD, r.y + r.height - 34.0),
                color: self.tokens.on_surface_muted,
                size: iced::Pixels(12.0),
                ..Text::default()
            });
        }

        frame.fill_text(Text {
            content: "Click a card to fling it up — compare Pop's overshoot vs Tap's crisp settle"
                .to_string(),
            position: Point::new(24.0, size.height - 30.0),
            color: self.tokens.on_surface_muted,
            size: iced::Pixels(13.0),
            ..Text::default()
        });

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.hit(bounds, cursor).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

/// 标题栏细线图标种类。
#[derive(Debug, Clone, Copy)]
pub enum Glyph {
    Minimize,
    Close,
}

/// 用描边线段自绘的细线图标（符合设计的 line 图标基调，且不引入任何字体/符号依赖）。
pub struct LineIcon {
    pub glyph: Glyph,
    pub color: Color,
}

impl<M> canvas::Program<M> for LineIcon {
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
            .with_width(1.4)
            .with_color(self.color)
            .with_line_cap(canvas::LineCap::Round);

        let inset = size.width * 0.30;
        let left = inset;
        let right = size.width - inset;
        let top = inset;
        let bottom = size.height - inset;
        let mid_y = size.height / 2.0;

        match self.glyph {
            Glyph::Minimize => {
                frame.stroke(
                    &Path::line(Point::new(left, mid_y), Point::new(right, mid_y)),
                    stroke,
                );
            }
            Glyph::Close => {
                frame.stroke(
                    &Path::line(Point::new(left, top), Point::new(right, bottom)),
                    stroke,
                );
                frame.stroke(
                    &Path::line(Point::new(right, top), Point::new(left, bottom)),
                    stroke,
                );
            }
        }

        vec![frame.into_geometry()]
    }
}
