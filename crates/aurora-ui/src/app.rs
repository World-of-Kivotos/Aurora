//! 应用外壳：iced 0.14 无边框圆角不透明窗口（纯白/纯深底）+ 自绘标题栏 + 左侧可展开图标导航栏 +
//! 右内容区。持有后端门面句柄、全局路由与消息，负责把帧与消息转发给当前页、把页面 Task 用 `.map`
//! 包回全局。页面 agent 不碰本文件：各页只在 `pages/<page>.rs` 内按契约填充。

use std::sync::Arc;

use aurora_core::Aurora;
use iced::time::Instant;
use iced::widget::{Space, button, column, container, mouse_area, row, stack, text};
use iced::{
    Alignment, Background, Border, Element, Fill, Length, Shadow, Size, Subscription, Task, Theme,
    Vector, window,
};

use crate::anim::{self, Animated};
use crate::background::Background as AuroraBackground;
use crate::ctx::Ctx;
use crate::pages;
use crate::theme::{self, Mode, Tokens};
use crate::widgets::{Icon, icon, nav_item};

/// 自绘标题栏高度。
const TITLE_H: f32 = 44.0;
/// 导航栏收起宽度（仅图标）。
const NAV_COLLAPSED: f32 = 64.0;
/// 导航栏展开宽度（图标 + 文字）。
const NAV_EXPANDED: f32 = 208.0;

/// 应用入口：装配 iced application 并运行。
pub fn run() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title("Aurora")
        .theme(App::theme)
        .style(App::style)
        .subscription(App::subscription)
        .default_font(theme::FONT_FAMILY)
        .antialiasing(true)
        .window(window_settings())
        .run()
}

/// 全局路由：五个一级页面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Accounts,
    Versions,
    Mods,
    Settings,
}

impl Screen {
    /// 导航顺序（也是导航栏从上到下的排列）。
    pub const ALL: [Screen; 5] = [
        Screen::Home,
        Screen::Accounts,
        Screen::Versions,
        Screen::Mods,
        Screen::Settings,
    ];

    /// 导航项与页面标题文案。
    fn label(self) -> &'static str {
        match self {
            Screen::Home => "主页",
            Screen::Accounts => "账户",
            Screen::Versions => "版本",
            Screen::Mods => "资源",
            Screen::Settings => "设置",
        }
    }

    /// 导航项图标。
    fn icon(self) -> Icon {
        match self {
            Screen::Home => Icon::Home,
            Screen::Accounts => Icon::Account,
            Screen::Versions => Icon::Versions,
            Screen::Mods => Icon::Mods,
            Screen::Settings => Icon::Settings,
        }
    }
}

/// 后端就绪后的外壳内容：门面句柄 + 当前页 + 各页状态。
struct Ready {
    core: Arc<Aurora>,
    screen: Screen,
    pages: Pages,
}

/// 各页状态集合。
#[derive(Default)]
struct Pages {
    home: pages::home::State,
    accounts: pages::accounts::State,
    versions: pages::versions::State,
    mods: pages::mods::State,
    settings: pages::settings::State,
}

/// 应用装配阶段：后端异步载入前显示加载屏，失败显示错误屏，成功进入完整外壳。
enum Stage {
    Loading,
    Failed(String),
    // Ready 变体远大于其它变体（各页 State 累积逾千字节），装箱抹平尺寸差避免 large_enum_variant。
    Ready(Box<Ready>),
}

/// 应用状态。
pub struct App {
    mode: Mode,
    /// 主窗口 Id（`open_events` 捕获后用于最小化/关闭/拖拽等窗口命令）。
    window: Option<window::Id>,
    background: AuroraBackground,
    /// 导航栏是否处于展开态（hover 驱动）。
    nav_expanded: bool,
    /// 导航栏宽度弹簧（收起 <-> 展开，Aurora 默认手感）。
    nav_width: Animated,
    /// 页面入场进度弹簧（0->1，略软手感，驱动内容轻微滑入）。
    page_enter: Animated,
    /// 上一帧时间戳，用于按相邻 Instant 求真实 dt。
    last_tick: Option<Instant>,
    stage: Stage,
}

/// 全局消息：外壳消息 + 每页一个包装变体。
///
/// 不派生 `Debug`：`CoreLoaded` 携带 `Arc<Aurora>`，而后端门面不实现 `Debug`。iced 不要求 Message: Debug。
#[derive(Clone)]
pub enum Message {
    /// 帧时钟：携带本帧 Instant，与上一帧求真实 dt。
    Tick(Instant),
    /// 切换到某页。
    Navigate(Screen),
    /// 导航栏展开/收起（hover 进入/离开）。
    NavExpand(bool),
    /// 切换亮/暗主题。
    ToggleTheme,
    Minimize,
    Close,
    DragWindow,
    /// 主窗口打开，携带其 Id。
    WindowOpened(window::Id),
    /// 后端门面载入完成（成功句柄或错误说明）。
    CoreLoaded(Result<Arc<Aurora>, String>),
    /// 主页消息。
    Home(pages::home::Message),
    /// 账户页消息。
    Accounts(pages::accounts::Message),
    /// 版本页消息。
    Versions(pages::versions::Message),
    /// 资源页消息。
    Mods(pages::mods::Message),
    /// 设置页消息。
    Settings(pages::settings::Message),
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let app = App {
            mode: Mode::Light,
            window: None,
            background: AuroraBackground::new(),
            nav_expanded: false,
            nav_width: Animated::new(NAV_COLLAPSED, anim::aurora_rail()),
            page_enter: Animated::new(1.0, anim::aurora_enter()),
            last_tick: None,
            stage: Stage::Loading,
        };
        // 应用级异步范式：后台载入后端门面，完成后 CoreLoaded 落地。
        (app, Task::perform(load_core(), Message::CoreLoaded))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick(now) => {
                let dt = match self.last_tick {
                    Some(prev) => now.saturating_duration_since(prev).as_secs_f32(),
                    None => 0.0,
                };
                self.last_tick = Some(now);
                self.nav_width.step(dt);
                self.page_enter.step(dt);
                // 把帧广播给当前页推进其动画。
                if let Stage::Ready(ready) = &mut self.stage {
                    let ctx = Ctx {
                        core: ready.core.clone(),
                        mode: self.mode,
                    };
                    tick_page(ready.screen, &mut ready.pages, dt, &ctx);
                }
                Task::none()
            }
            Message::Navigate(screen) => {
                if let Stage::Ready(ready) = &mut self.stage {
                    ready.screen = screen;
                }
                // 重置入场弹簧从 0 弹到 1，驱动内容滑入；重置计时基准避免首帧大 dt。
                self.page_enter = Animated::new(0.0, anim::aurora_enter());
                self.page_enter.set(1.0);
                self.last_tick = None;
                Task::none()
            }
            Message::NavExpand(expanded) => {
                self.nav_expanded = expanded;
                self.nav_width
                    .set(if expanded { NAV_EXPANDED } else { NAV_COLLAPSED });
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
                Task::none()
            }
            Message::CoreLoaded(result) => {
                match result {
                    Ok(core) => {
                        let (pages, task) = init_pages(&core, self.mode);
                        self.stage = Stage::Ready(Box::new(Ready {
                            core,
                            screen: Screen::Home,
                            pages,
                        }));
                        // 入场动画。
                        self.page_enter = Animated::new(0.0, anim::aurora_enter());
                        self.page_enter.set(1.0);
                        self.last_tick = None;
                        task
                    }
                    Err(error) => {
                        self.stage = Stage::Failed(error);
                        Task::none()
                    }
                }
            }
            Message::Home(msg) => self.forward(|ready, ctx| {
                pages::home::update(&mut ready.pages.home, msg, ctx).map(Message::Home)
            }),
            Message::Accounts(msg) => self.forward(|ready, ctx| {
                pages::accounts::update(&mut ready.pages.accounts, msg, ctx).map(Message::Accounts)
            }),
            Message::Versions(msg) => self.forward(|ready, ctx| {
                pages::versions::update(&mut ready.pages.versions, msg, ctx).map(Message::Versions)
            }),
            Message::Mods(msg) => self.forward(|ready, ctx| {
                pages::mods::update(&mut ready.pages.mods, msg, ctx).map(Message::Mods)
            }),
            Message::Settings(msg) => self.forward(|ready, ctx| {
                pages::settings::update(&mut ready.pages.settings, msg, ctx).map(Message::Settings)
            }),
        }
    }

    /// 把一条页面消息转发给 Ready 状态下的对应页（构造 Ctx，运行给定闭包）。非 Ready 时丢弃。
    fn forward<F>(&mut self, run: F) -> Task<Message>
    where
        F: FnOnce(&mut Ready, &Ctx) -> Task<Message>,
    {
        let mode = self.mode;
        if let Stage::Ready(ready) = &mut self.stage {
            let ctx = Ctx {
                core: ready.core.clone(),
                mode,
            };
            run(ready, &ctx)
        } else {
            Task::none()
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let shell_animating = !self.nav_width.settled() || !self.page_enter.settled();
        let page_animating = match &self.stage {
            Stage::Ready(ready) => page_animating(ready.screen, &ready.pages),
            _ => false,
        };

        // 收敛即停：仅在外壳或当前页有动画时挂帧订阅。用 window::frames() 与 vsync 同步、帧距均匀。
        let ticks = if shell_animating || page_animating {
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
        let background = self.background.view(tokens);

        let foreground: Element<'_, Message> = match &self.stage {
            Stage::Loading => center_message("正在初始化 Aurora…", tokens),
            Stage::Failed(error) => center_message(&format!("初始化失败：{error}"), tokens),
            Stage::Ready(ready) => self.shell(ready, tokens),
        };

        let body = column![self.title_bar(tokens), foreground];
        stack![background, body].into()
    }

    /// Ready 状态的完整外壳：内容层 + 浮在其上的导航覆盖层。
    ///
    /// 导航栏做成 `stack` 覆盖层而非并排列：内容层左侧只用一枚固定 [`NAV_COLLAPSED`] 宽的占位（`Space`）
    /// 让内容从收起宽度之后起排，导航展开/收起时内容**不再逐帧重排**（省去整棵内容子树的重排，稳住帧
    /// 率）；导航面板按弹簧宽度浮在内容之上展开。hover 命中区即导航面板自身：收起 64、展开 208 两个边界
    /// 天然构成滞回（须移入 64 才展开、移出 208 才收起），配合脆弹簧（[`anim::aurora_rail`] 零过冲）不会
    /// 出现命中边界来回扫过光标的抖动。
    fn shell<'a>(&'a self, ready: &'a Ready, tokens: Tokens) -> Element<'a, Message> {
        let base = row![
            Space::new()
                .width(Length::Fixed(NAV_COLLAPSED))
                .height(Length::Fill),
            self.content(ready),
        ]
        .width(Fill)
        .height(Fill);

        stack![base, self.nav_rail(ready, tokens)]
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// 左侧可展开导航栏（外壳的覆盖层）。宽度由弹簧驱动；hover 进入/离开切换展开态。面板底为冷浅灰（比纯白
    /// 内容区偏灰一档，好让白色选中胶囊浮起）+ 右侧柔和阴影，展开时干净盖住其下内容并呈浮起感。
    fn nav_rail<'a>(&'a self, ready: &'a Ready, tokens: Tokens) -> Element<'a, Message> {
        let width = self.nav_width.value();
        let reveal = ((width - NAV_COLLAPSED) / (NAV_EXPANDED - NAV_COLLAPSED)).clamp(0.0, 1.0);

        let items = Screen::ALL.map(|screen| {
            nav_item(
                screen.icon(),
                screen.label(),
                ready.screen == screen,
                reveal,
                tokens,
                Message::Navigate(screen),
            )
        });

        let mut list = column![].spacing(theme::SPACE_XS).padding(theme::SPACE_SM);
        for item in items {
            list = list.push(item);
        }

        let rail = container(list)
            .width(Length::Fixed(width))
            .height(Fill)
            .style(move |_theme: &Theme| container::Style {
                background: Some(Background::Color(tokens.nav_rail)),
                border: Border {
                    color: tokens.elevated_border,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                // 右侧柔和投影，让展开的面板从内容上「浮起」，界限清晰。
                shadow: Shadow {
                    color: tokens.shadow,
                    offset: Vector::new(3.0, 0.0),
                    blur_radius: 16.0,
                },
                ..container::Style::default()
            });

        mouse_area(rail)
            .on_enter(Message::NavExpand(true))
            .on_exit(Message::NavExpand(false))
            .into()
    }

    /// 右内容区：当前页 view 外套入场位移（float translate，不触发重排）。
    fn content<'a>(&'a self, ready: &'a Ready) -> Element<'a, Message> {
        let ctx = Ctx {
            core: ready.core.clone(),
            mode: self.mode,
        };
        let page = page_view(ready, &ctx);
        // 入场：从右侧 24px 滑入到 0，随 page_enter 0->1 收敛。
        let shift = (1.0 - self.page_enter.value()) * 24.0;
        let slid: Element<'a, Message> = iced::widget::float(page)
            .translate(move |_content, _viewport| Vector::new(shift, 0.0))
            .into();
        container(slid).width(Fill).height(Fill).into()
    }

    fn title_bar(&self, tokens: Tokens) -> Element<'_, Message> {
        let name = mouse_area(
            container(
                text("Aurora")
                    .size(theme::TEXT_BODY)
                    .color(tokens.title_text),
            )
            .padding([0.0, theme::SPACE_MD])
            .center_y(Fill)
            .width(Fill)
            .height(Fill),
        )
        .on_press(Message::DragWindow);

        let theme_btn = button(
            text(match self.mode {
                Mode::Dark => "亮色",
                Mode::Light => "暗色",
            })
            .size(theme::TEXT_CAPTION)
            .color(tokens.title_text),
        )
        .on_press(Message::ToggleTheme)
        .padding([theme::SPACE_XS, theme::SPACE_SM])
        .style(titlebar_button);

        let controls = row![
            theme_btn,
            icon_button(Icon::Minimize, tokens, Message::Minimize),
            icon_button(Icon::Close, tokens, Message::Close),
        ]
        .align_y(Alignment::Center)
        .spacing(theme::SPACE_XS);

        container(
            row![name, controls]
                .align_y(Alignment::Center)
                .height(Fill),
        )
        .width(Fill)
        .height(Length::Fixed(TITLE_H))
        .padding([0.0, theme::SPACE_SM])
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(tokens.elevated)),
            border: Border {
                color: tokens.elevated_border,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
    }

    fn theme(&self) -> Theme {
        self.mode.iced_theme()
    }

    fn style(&self, _theme: &Theme) -> iced::theme::Style {
        // 窗口底为不透明纯色（Light 纯白 / Dark 纯深底），无桌面穿透；随明暗走令牌基底色。
        let tokens = theme::tokens(self.mode);
        iced::theme::Style {
            background_color: tokens.bg_from,
            text_color: tokens.title_text,
        }
    }

    /// 优先用已知窗口 Id 直接下发命令；未知时回退 `window::latest()` 现查。
    fn window_task<F>(&self, action: F) -> Task<Message>
    where
        F: Fn(window::Id) -> Task<Message> + Send + 'static,
    {
        match self.window {
            Some(id) => action(id),
            None => window::latest().and_then(action),
        }
    }
}

/// 后台载入后端门面：默认数据目录读配置、建共享 HTTP 客户端。错误在边界转成 String（Message 需 Clone）。
async fn load_core() -> Result<Arc<Aurora>, String> {
    Aurora::load()
        .await
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

/// 初始化各页状态并聚合首个副作用（各页 init 的 Task 各自 .map 回全局后 batch）。
fn init_pages(core: &Arc<Aurora>, mode: Mode) -> (Pages, Task<Message>) {
    let ctx = Ctx {
        core: core.clone(),
        mode,
    };
    let (home, home_task) = pages::home::init(&ctx);
    let (accounts, accounts_task) = pages::accounts::init(&ctx);
    let (versions, versions_task) = pages::versions::init(&ctx);
    let (mods, mods_task) = pages::mods::init(&ctx);
    let (settings, settings_task) = pages::settings::init(&ctx);

    let pages = Pages {
        home,
        accounts,
        versions,
        mods,
        settings,
    };
    let task = Task::batch([
        home_task.map(Message::Home),
        accounts_task.map(Message::Accounts),
        versions_task.map(Message::Versions),
        mods_task.map(Message::Mods),
        settings_task.map(Message::Settings),
    ]);
    (pages, task)
}

/// 当前页 view，Message 经 `Element::map` 包回全局。
fn page_view<'a>(ready: &'a Ready, ctx: &Ctx) -> Element<'a, Message> {
    match ready.screen {
        Screen::Home => pages::home::view(&ready.pages.home, ctx).map(Message::Home),
        Screen::Accounts => {
            pages::accounts::view(&ready.pages.accounts, ctx).map(Message::Accounts)
        }
        Screen::Versions => {
            pages::versions::view(&ready.pages.versions, ctx).map(Message::Versions)
        }
        Screen::Mods => pages::mods::view(&ready.pages.mods, ctx).map(Message::Mods),
        Screen::Settings => {
            pages::settings::view(&ready.pages.settings, ctx).map(Message::Settings)
        }
    }
}

/// 把帧广播给当前页推进动画。
fn tick_page(screen: Screen, pages: &mut Pages, dt: f32, ctx: &Ctx) {
    match screen {
        Screen::Home => pages::home::tick(&mut pages.home, dt, ctx),
        Screen::Accounts => pages::accounts::tick(&mut pages.accounts, dt, ctx),
        Screen::Versions => pages::versions::tick(&mut pages.versions, dt, ctx),
        Screen::Mods => pages::mods::tick(&mut pages.mods, dt, ctx),
        Screen::Settings => pages::settings::tick(&mut pages.settings, dt, ctx),
    }
}

/// 当前页是否仍有动画未收敛。
fn page_animating(screen: Screen, pages: &Pages) -> bool {
    match screen {
        Screen::Home => pages::home::animating(&pages.home),
        Screen::Accounts => pages::accounts::animating(&pages.accounts),
        Screen::Versions => pages::versions::animating(&pages.versions),
        Screen::Mods => pages::mods::animating(&pages.mods),
        Screen::Settings => pages::settings::animating(&pages.settings),
    }
}

/// 居中的一句提示（加载/错误屏用）。
fn center_message<'a>(message: &str, tokens: Tokens) -> Element<'a, Message> {
    iced::widget::center(
        text(message.to_owned())
            .size(theme::TEXT_HEADING)
            .color(tokens.on_surface),
    )
    .into()
}

/// 标题栏图标按钮：细线图标 + 悬停弱高光。
fn icon_button(glyph: Icon, tokens: Tokens, message: Message) -> Element<'static, Message> {
    button(icon(glyph, 16.0, tokens.icon))
        .on_press(message)
        .padding(theme::SPACE_SM)
        .style(titlebar_button)
        .into()
}

/// 标题栏按钮样式：无底、悬停/按下叠一层弱高光、细圆角。
fn titlebar_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let mut style = button::text(theme, status);
    style.border = Border {
        radius: theme::RADIUS_SM.into(),
        ..style.border
    };
    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = Some(Background::Color(palette.background.weak.color));
    }
    style
}

fn window_settings() -> window::Settings {
    let mut settings = window::Settings {
        size: Size::new(1000.0, 640.0),
        min_size: Some(Size::new(760.0, 480.0)),
        resizable: true,
        decorations: false,
        // 不透明窗口：内容自绘纯白/纯深底，无桌面穿透。无边框圆角与投影走下面的 DWM 设置。
        transparent: false,
        ..window::Settings::default()
    };

    // 无边框下的圆角与投影走 DWM。CornerPreference::Round 需 Win11 Build 22000+，旧系统静默退化为直角。
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
