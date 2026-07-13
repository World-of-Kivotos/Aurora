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
use iced::time::{self, Duration, Instant};
use iced::widget::{button, canvas, container, mouse_area, row, stack, text};
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

struct Aurora {
    mode: Mode,
    cards: Vec<Card>,
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
        // 五张卡片对应五枚预设，点击后同样抬起 96px：从 Tap 的零过冲到 Pop 的大过冲，
        // 肉眼直接对比落定手感的完整梯度。
        let cards = vec![
            Card::new("Tap", "no overshoot", anim::TAP),
            Card::new("Settle", "bounce 0.10", anim::SETTLE),
            Card::new("Soft", "bounce 0.12", anim::SOFT),
            Card::new("Morph", "bounce 0.14", anim::MORPH),
            Card::new("Pop", "bounce 0.20", anim::POP),
        ];

        (
            Self {
                mode: Mode::Dark,
                cards,
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

    fn subscription(&self) -> Subscription<Message> {
        // 收敛即停：只要还有弹簧未静止就保持 16ms 帧订阅，全部静止后退订以省电。
        let animating = self.cards.iter().any(|card| !card.lift.settled());
        let ticks = if animating {
            time::every(Duration::from_millis(16)).map(Message::Tick)
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

        // 画布铺满整窗（含标题栏区域，让极光背景连续），标题栏作为覆盖层浮在顶部。
        stack![scene, self.title_bar(tokens)].into()
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
