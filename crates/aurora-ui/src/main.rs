//! Aurora 前端「可运行验收切片」。
//!
//! 目标是给 iced 0.14 + 无边框圆角窗口 + 两参弹簧动效做技术验证（de-risk），
//! 不是完整前端。四块拼图：
//! - [`anim`]  弹簧内核（半隐式欧拉 + 五枚预设 + 收敛停订阅）。
//! - [`theme`] 亮/暗主题令牌。
//! - [`scene`] 极光渐变背景 + 弹簧卡片画布。
//! - 本文件  iced 应用外壳：无边框圆角窗口、自绘标题栏、时间订阅驱动、亚克力尝试。

mod anim;
mod scene;
mod theme;

use iced::alignment;
use iced::time::Instant;
use iced::widget::{button, canvas, column, container, mouse_area, row, slider, stack, text};
use iced::window;
use iced::{Background, Border, Element, Fill, Size, Subscription, Task, Theme};

use scene::{Card, DemoScene, Glyph, LineIcon};
use theme::{Mode, Tokens};

fn main() -> iced::Result {
    iced::application(Aurora::boot, Aurora::update, Aurora::view)
        .title("Aurora")
        .theme(|state: &Aurora| state.mode.iced_theme())
        .style(Aurora::style)
        .subscription(Aurora::subscription)
        .window(window_settings())
        .run()
}

/// 亚克力应用状态，仅用于在标题栏回显本次窗口句柄 + window-vibrancy 的结果。
#[derive(Debug, Clone, Copy)]
enum AcrylicStatus {
    Pending,
    Active,
    Unsupported,
}

impl AcrylicStatus {
    fn label(self) -> &'static str {
        match self {
            AcrylicStatus::Pending => "pending",
            AcrylicStatus::Active => "on",
            AcrylicStatus::Unsupported => "off",
        }
    }
}

/// 弹跳强度可调区间：0=临界阻尼零过冲，上限 0.8 已相当弹。
const BOUNCE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.8;
/// 到位时长（秒）可调区间：0.15 干脆、0.6 舒缓。
const DURATION_RANGE: std::ops::RangeInclusive<f32> = 0.15..=0.6;

struct Aurora {
    mode: Mode,
    cards: Vec<Card>,
    /// 弹跳强度（1−阻尼比）。见 [`BOUNCE_RANGE`]。滑条实时驱动全体卡片弹簧。
    bounce: f32,
    /// 到位时长（秒）。见 [`DURATION_RANGE`]。
    duration: f32,
    /// 主窗口 Id。由 `open_events` 捕获后用于窗口命令与亚克力应用。
    window: Option<window::Id>,
    acrylic: AcrylicStatus,
    /// 上一帧时间戳，用于按相邻 Instant 求真实 dt。
    last_tick: Option<Instant>,
}

#[derive(Debug, Clone)]
enum Message {
    /// 帧时钟：携带本帧 Instant，与上一帧求真实 dt。
    Tick(Instant),
    /// 第 n 张卡片被点击，切换其抬起/落回。
    CardClicked(usize),
    /// 弹跳强度滑条变化：换算并即时写入全体卡片弹簧。
    BounceChanged(f32),
    /// 到位时长滑条变化：换算并即时写入全体卡片弹簧。
    DurationChanged(f32),
    ToggleTheme,
    Minimize,
    Close,
    DragWindow,
    /// 主窗口打开，携带其 Id。
    WindowOpened(window::Id),
    /// 亚克力应用结果（能否拿到句柄并应用成功）。
    AcrylicResult(bool),
}

impl Aurora {
    fn boot() -> (Self, Task<Message>) {
        // playground 模式：三张卡片共享同一套由 bounce/duration 实时换算的弹簧，点任意一张
        // 都用当前手感抬起 96px。默认给一个略带弹性的手感（bounce 0.35 / 0.30s）作起点。
        let bounce = 0.35;
        let duration = 0.30;
        let params = anim::params_from(bounce, duration);
        let cards = vec![
            Card::new("One", "tap to fling", params),
            Card::new("Two", "tap to fling", params),
            Card::new("Three", "tap to fling", params),
        ];

        (
            Self {
                mode: Mode::Dark,
                cards,
                bounce,
                duration,
                window: None,
                acrylic: AcrylicStatus::Pending,
                last_tick: None,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick(now) => {
                let dt = match self.last_tick {
                    Some(prev) => now.saturating_duration_since(prev).as_secs_f32(),
                    None => 0.0,
                };
                self.last_tick = Some(now);
                for card in &mut self.cards {
                    card.lift.step(dt);
                }
                Task::none()
            }
            Message::CardClicked(index) => {
                if let Some(card) = self.cards.get_mut(index) {
                    card.toggle();
                }
                // 新一轮动画开始时重置计时基准，避免用「上次动画结束到现在」的大间隔
                // 算首帧 dt 造成跳变。保留弹簧速度，惯性不受影响。
                self.last_tick = None;
                Task::none()
            }
            Message::BounceChanged(value) => {
                self.bounce = value;
                self.apply_spring_params();
                Task::none()
            }
            Message::DurationChanged(value) => {
                self.duration = value;
                self.apply_spring_params();
                Task::none()
            }
            Message::ToggleTheme => {
                self.mode = self.mode.toggled();
                Task::none()
            }
            Message::Minimize => self.window_task(|id| window::minimize(id, true)),
            Message::Close => self.window_task(window::close),
            Message::DragWindow => self.window_task(window::drag),
            Message::WindowOpened(id) => {
                self.window = Some(id);
                // 关键验证点：iced 0.14 通过 window::run 把 &dyn Window（实现
                // HasWindowHandle）交给闭包，据此拿到原生句柄交给 window-vibrancy。
                let tint = acrylic_tint(self.mode);
                window::run(id, move |handle| {
                    window_vibrancy::apply_acrylic(handle, Some(tint)).is_ok()
                })
                .map(Message::AcrylicResult)
            }
            Message::AcrylicResult(applied) => {
                self.acrylic = if applied {
                    AcrylicStatus::Active
                } else {
                    AcrylicStatus::Unsupported
                };
                Task::none()
            }
        }
    }

    /// 把当前 bounce/duration 换算出的 k/c 写入全体卡片弹簧。只改 k、c 两枚参数，刻意
    /// 保留 current/velocity/target：正在飞行中的卡片会当帧改变手感（可中断、不跳变），
    /// 静止的卡片则在下次点击时以新手感弹起。
    fn apply_spring_params(&mut self) {
        let params = anim::params_from(self.bounce, self.duration);
        for card in &mut self.cards {
            card.lift.k = params.k;
            card.lift.c = params.c;
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // 收敛即停：仅在有弹簧未静止时挂帧订阅，全部静止后退订以省电。
        let animating = self.cards.iter().any(|card| !card.lift.settled());
        let ticks = if animating {
            // 用 window::frames() 而非 time::every(16ms) 驱动动画：前者每呈现一帧触发一次，
            // 随合成器按显示器刷新率跑并与 vsync 同步、帧距均匀；后者在 Windows 上受系统
            // 定时器 ~15.6ms 粒度限制，请求 16ms 被上取整到 ~31ms（~32fps）且与刷新不同步。
            // 产出的 Instant 为本帧 RedrawRequested 时刻，直接喂给 Tick 与 last_tick 求真实 dt。
            window::frames().map(Message::Tick)
        } else {
            Subscription::none()
        };

        Subscription::batch([
            window::open_events().map(Message::WindowOpened),
            ticks,
        ])
    }

    fn view(&self) -> Element<'_, Message> {
        let tokens = theme::tokens(self.mode);

        let scene = canvas(DemoScene {
            cards: self.cards.as_slice(),
            tokens,
        })
        .width(Fill)
        .height(Fill);

        // 画布占据上方全部剩余空间、调参面板固定在底部，二者纵向排布互不遮挡；卡片由画布
        // 在其收缩后的 bounds 内自动居中，不会压到面板。标题栏仍作为覆盖层浮在最顶部，
        // 让极光背景在其下连续。
        let body = column![scene, self.controls(tokens)];
        stack![body, self.title_bar(tokens)].into()
    }

    fn style(&self, _theme: &Theme) -> iced::theme::Style {
        let tokens = theme::tokens(self.mode);
        iced::theme::Style {
            background_color: tokens.window_base,
            text_color: tokens.title_text,
        }
    }

    /// 优先用已知窗口 Id 直接下发命令（拖拽需尽量贴近按下时刻）；未知时回退到
    /// `window::latest()` 现查。
    fn window_task<F>(&self, action: F) -> Task<Message>
    where
        F: Fn(window::Id) -> Task<Message> + Send + 'static,
    {
        match self.window {
            Some(id) => action(id),
            None => window::latest().and_then(action),
        }
    }

    fn title_bar(&self, tokens: Tokens) -> Element<'_, Message> {
        let name = mouse_area(
            container(
                text(format!("Aurora   ·   acrylic: {}", self.acrylic.label()))
                    .size(14.0)
                    .color(tokens.title_text),
            )
            .padding([0.0, 16.0])
            .center_y(Fill)
            .width(Fill)
            .height(Fill),
        )
        .on_press(Message::DragWindow);

        let theme_btn = button(text("Theme").size(12.0).color(tokens.title_text))
            .on_press(Message::ToggleTheme)
            .padding([6.0, 12.0])
            .style(titlebar_button);

        let controls = row![
            theme_btn,
            icon_button(Glyph::Minimize, tokens, Message::Minimize),
            icon_button(Glyph::Close, tokens, Message::Close),
        ]
        .align_y(alignment::Vertical::Center)
        .spacing(2);

        container(
            row![name, controls]
                .align_y(alignment::Vertical::Center)
                .height(Fill),
        )
        .width(Fill)
        .height(scene::TITLE_H)
        .padding([0.0, 6.0])
        .into()
    }

    /// 底部实时调参面板：两枚滑条（主=弹跳强度，次=到位时长）+ 一行数字回显。半透明面
    /// 叠在窗口底色上、与卡片同色系，读作一条毛玻璃控制条；固定在窗口底部，不遮挡卡片。
    fn controls(&self, tokens: Tokens) -> Element<'_, Message> {
        let bounce_row = slider_row(
            "Bounce (spring strength)",
            slider(BOUNCE_RANGE, self.bounce, Message::BounceChanged).step(0.01_f32),
            format!("{:.2}", self.bounce),
            tokens,
        );
        let duration_row = slider_row(
            "Duration (seconds)",
            slider(DURATION_RANGE, self.duration, Message::DurationChanged).step(0.01_f32),
            format!("{:.2}s", self.duration),
            tokens,
        );

        let readout = text(self.readout()).size(12.0).color(tokens.on_surface_muted);

        container(column![bounce_row, duration_row, readout].spacing(10.0))
            .width(Fill)
            .padding([14.0, 24.0])
            .style(move |_theme: &Theme| container::Style {
                background: Some(Background::Color(tokens.surface)),
                border: Border {
                    color: tokens.surface_border,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    /// 把「滑条数值」翻译成用户能对上手感的物理量：阻尼比 ζ=1−bounce、标准二阶系统超调
    /// 百分比、以及该过冲落在 96px 行程上的像素量，末尾附到位时长。全部保留 1 位小数。
    fn readout(&self) -> String {
        let mp = anim::overshoot_percent(self.bounce);
        format!(
            "bounce {:.1}   damping zeta {:.1}   overshoot {:.1}%  (~{:.1}px of {:.0}px)   settle ~{:.1}s",
            self.bounce,
            1.0 - self.bounce,
            mp,
            mp / 100.0 * scene::LIFT_DISTANCE,
            scene::LIFT_DISTANCE,
            self.duration,
        )
    }
}

/// 组装一枚滑条行：左固定宽标签 + 中间自适应滑条 + 右固定宽当前值。抽出以消除两行重复。
fn slider_row<'a>(
    label: &'static str,
    control: impl Into<Element<'a, Message>>,
    value: String,
    tokens: Tokens,
) -> Element<'a, Message> {
    row![
        text(label).size(13.0).color(tokens.title_text).width(190.0),
        control.into(),
        text(value).size(13.0).color(tokens.title_text).width(52.0),
    ]
    .spacing(16.0)
    .align_y(alignment::Vertical::Center)
    .into()
}

fn icon_button(glyph: Glyph, tokens: Tokens, message: Message) -> Element<'static, Message> {
    button(
        canvas(LineIcon {
            glyph,
            color: tokens.icon,
        })
        .width(16.0)
        .height(16.0),
    )
    .on_press(message)
    .padding(8.0)
    .style(titlebar_button)
    .into()
}

/// 标题栏按钮样式：无底、悬停/按下时叠一层弱高光；细圆角。
fn titlebar_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let mut style = button::text(theme, status);
    style.border = Border {
        radius: 8.0.into(),
        ..style.border
    };
    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = Some(Background::Color(palette.background.weak.color));
    }
    style
}

/// window-vibrancy 亚克力的着色（RGBA，A 为强度）。按明暗给不同底色。
fn acrylic_tint(mode: Mode) -> (u8, u8, u8, u8) {
    match mode {
        Mode::Dark => (18, 22, 40, 160),
        Mode::Light => (245, 247, 255, 160),
    }
}

fn window_settings() -> window::Settings {
    let mut settings = window::Settings {
        size: Size::new(1000.0, 640.0),
        min_size: Some(Size::new(760.0, 480.0)),
        resizable: true,
        decorations: false,
        transparent: false,
        ..window::Settings::default()
    };

    // 无边框下的圆角与投影走 DWM。CornerPreference::Round 需 Win11 Build 22000+，
    // 本机满足；旧系统会静默退化为直角，不影响其余功能。
    #[cfg(target_os = "windows")]
    {
        settings.platform_specific = window::settings::PlatformSpecific {
            corner_preference: window::settings::platform::CornerPreference::Round,
            undecorated_shadow: true,
            ..window::settings::PlatformSpecific::default()
        };
    }

    settings
}
