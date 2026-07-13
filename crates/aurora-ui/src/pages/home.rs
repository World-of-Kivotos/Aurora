//! 主页：极光背景上的核心落地页。
//!
//! 版式（自上而下）：页面头 -> 快速启动 hero 卡（大「开始游戏」按钮 + 就绪摘要 + 启动状态）->
//! 一行两卡（当前账户 / 版本选择）。进入时三张卡按 stagger 弹簧上滑落定；「开始游戏」点击时缩放从按下
//! 值弹回 1.0，给出一次弹性反馈。
//!
//! 后端经 `ctx.core`（`Arc<Aurora>`）异步取用：`init`/`刷新` 拉当前账户（`current_account`，Windows 下的
//! 加密账户库）与已安装版本（`list_installed`）；「开始游戏」调 `launch_offline` 触发一次离线启动，
//! aurora-core 内部会跑启动前检查（主类/客户端 jar/Java/路径），阻断项以 `PrecheckFailed` 冒泡成失败文案。
//!
//! 两点工程取舍（如实记录，非偷懒）：
//! 1. 页面契约只能产出本页 Message（外壳 `.map` 成 `Message::Home`），无法发起全局路由跳转。故「账户」
//!    卡不直接跳账户页，改为文案引导用户走左侧导航，并提供「刷新」重载账户/版本；真要点击跳页需外壳在
//!    契约里暴露页面可返回的导航请求。
//! 2. `launch_offline` 的 `events`/`log_tx` 是 `tokio::sync::mpsc` 通道，而 aurora-ui 未直接依赖 tokio，
//!    无法在本文件构造通道，故二者传 `None`：启动状态走「请求/响应」粗粒度（启动中/已启动/失败），暂不
//!    接细粒度阶段事件流。

use std::sync::Arc;

use aurora_core::{AccountType, Aurora, LaunchOptions};
use iced::alignment;
use iced::widget::{button, column, container, float, row, scrollable, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Task, Vector};

use crate::anim::{self, Animated};
use crate::ctx::Ctx;
use crate::theme::{self, Tokens};
use crate::widgets::{
    Icon, glass_card, icon, loading, page_header, primary_button, secondary_button, section_title,
    spring_press,
};

/// 入场卡片数量（hero / 账户 / 版本），决定 stagger 弹簧的枚数。
const CARD_COUNT: usize = 3;
/// 相邻卡片入场的时间错位（秒）。
const ENTER_STAGGER: f32 = 0.07;
/// 卡片入场的初始下移量（像素，随弹簧收敛到 0）。
const ENTER_SHIFT: f32 = 22.0;
/// 「开始游戏」按下反馈的起始缩放（从此值弹回 1.0）。
const LAUNCH_PRESS_FROM: f32 = 0.9;
/// 账户头像圆的边长（像素）。
const AVATAR_SIZE: f32 = 44.0;

/// 当前账户的展示摘要（Message 需 Clone，仅留可廉价克隆的展示字段）。
#[derive(Debug, Clone)]
pub struct AccountSummary {
    name: String,
    account_type: AccountType,
}

/// 一次成功启动的结果摘要。
#[derive(Debug, Clone)]
pub struct LaunchInfo {
    pid: Option<u32>,
    version: String,
}

/// `init`/`刷新` 拉取的落地页快照（账户可空表示未选择；版本可空表示未安装）。
#[derive(Debug, Clone)]
pub struct HomeSnapshot {
    account: Option<AccountSummary>,
    versions: Vec<String>,
}

/// 启动状态机。
#[derive(Debug)]
enum LaunchStatus {
    /// 未启动。
    Idle,
    /// 已发起 `launch_offline`，等待结果。
    Launching,
    /// 进程已拉起。
    Running(LaunchInfo),
    /// 启动失败（含启动前检查阻断项文案）。
    Failed(String),
}

/// 页面状态。
#[derive(Debug)]
pub struct State {
    /// 账户/版本快照是否正在加载。
    loading: bool,
    /// 快照加载错误（账户库或版本扫描失败时的说明）。
    error: Option<String>,
    /// 当前账户摘要。
    account: Option<AccountSummary>,
    /// 已安装版本 id 列表。
    versions: Vec<String>,
    /// 当前选中的版本 id。
    selected: Option<String>,
    /// 启动状态。
    launch: LaunchStatus,
    /// 「开始游戏」按压缩放（点击时从按下值弹回 1.0）。
    launch_scale: Animated,
    /// 三张卡片的入场进度弹簧（0->1）。
    cards: [Animated; CARD_COUNT],
    /// 入场累计时长，用于按 stagger 逐张触发。
    enter_elapsed: f32,
    /// 入场是否全部落定（决定是否继续挂帧）。
    enter_done: bool,
}

impl Default for State {
    fn default() -> Self {
        let enter = anim::aurora_enter();
        Self {
            loading: true,
            error: None,
            account: None,
            versions: Vec::new(),
            selected: None,
            launch: LaunchStatus::Idle,
            launch_scale: Animated::new(1.0, anim::aurora_press()),
            cards: [Animated::new(0.0, enter); CARD_COUNT],
            enter_elapsed: 0.0,
            enter_done: false,
        }
    }
}

impl State {
    /// 保持已选版本；若其已不在列表则退回首个（列表空则清空）。
    fn reconcile_selection(&mut self) {
        let keep = self
            .selected
            .as_ref()
            .is_some_and(|s| self.versions.iter().any(|v| v == s));
        if !keep {
            self.selected = self.versions.first().cloned();
        }
    }

    /// 是否满足启动前置：有账户、有选中版本、且不在启动中。
    fn ready(&self) -> bool {
        self.account.is_some()
            && self.selected.is_some()
            && !matches!(self.launch, LaunchStatus::Launching)
    }
}

/// 页面消息。
#[derive(Debug, Clone)]
pub enum Message {
    /// 账户/版本快照加载完成。
    Loaded(Result<HomeSnapshot, String>),
    /// 重新拉取账户与版本。
    Reload,
    /// 选中某个版本。
    SelectVersion(String),
    /// 点击「开始游戏」。
    Launch,
    /// 启动结果返回。
    Launched(Result<LaunchInfo, String>),
}

/// 构造页面状态并立即拉取账户/版本快照（主页是默认页，进入即需要这些信息）。
pub fn init(ctx: &Ctx) -> (State, Task<Message>) {
    let core = ctx.core.clone();
    (State::default(), Task::perform(load(core), Message::Loaded))
}

/// 处理页面消息。
pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Reload => {
            state.loading = true;
            state.error = None;
            let core = ctx.core.clone();
            Task::perform(load(core), Message::Loaded)
        }
        Message::Loaded(result) => {
            state.loading = false;
            match result {
                Ok(snapshot) => {
                    state.account = snapshot.account;
                    state.versions = snapshot.versions;
                    state.reconcile_selection();
                }
                Err(message) => state.error = Some(message),
            }
            Task::none()
        }
        Message::SelectVersion(id) => {
            state.selected = Some(id);
            Task::none()
        }
        Message::Launch => {
            // 就绪判定不成立时按钮已禁用；此处再兜一层，避免竞态下空转出错。
            let (Some(account), Some(version)) = (&state.account, &state.selected) else {
                return Task::none();
            };
            let player = account.name.clone();
            let version = version.clone();
            state.launch = LaunchStatus::Launching;
            // 点击反馈：缩放从按下值弹回 1.0，默认手感带轻微过冲。
            state.launch_scale = Animated::new(LAUNCH_PRESS_FROM, anim::aurora());
            state.launch_scale.set(1.0);
            let core = ctx.core.clone();
            Task::perform(launch(core, version, player), Message::Launched)
        }
        Message::Launched(result) => {
            state.launch = match result {
                Ok(info) => LaunchStatus::Running(info),
                Err(message) => LaunchStatus::Failed(message),
            };
            Task::none()
        }
    }
}

/// 拉取账户/版本快照。账户库读取（Windows 加密库）为同步调用，在此异步块内一并完成；两处失败都在边界
/// 转成 `String` 上浮（`CoreError` 不 Clone），空账户/空版本是合法状态、不作错误。
async fn load(core: Arc<Aurora>) -> Result<HomeSnapshot, String> {
    let account = core
        .current_account()
        .map_err(|error| error.to_string())?
        .map(|account| AccountSummary {
            name: account.name,
            account_type: account.account_type,
        });
    let scan = core.list_installed().await.map_err(|error| error.to_string())?;
    let versions = scan.versions.into_iter().map(|version| version.id).collect();
    Ok(HomeSnapshot { account, versions })
}

/// 以离线方式启动选中版本。`launch_offline` 内部完成账户组装、版本合并、Java 解析与启动前检查；返回的
/// `GameSession` 内含 tokio `Child`（默认不随 drop 杀进程），故取到 pid 后让会话在此丢弃，游戏继续运行。
async fn launch(core: Arc<Aurora>, version: String, player: String) -> Result<LaunchInfo, String> {
    let options = LaunchOptions::default();
    let session = core
        .launch_offline(&version, &player, &options, None, None)
        .await
        .map_err(|error| error.to_string())?;
    Ok(LaunchInfo {
        pid: session.id(),
        version,
    })
}

/// 渲染页面。整页套一层竖向 `scrollable`，小窗口下内容超高可滚动、不裁切。
pub fn view<'a>(state: &'a State, ctx: &Ctx) -> Element<'a, Message> {
    let tokens = ctx.tokens();

    let hero = entrance(hero_card(state, tokens), card_offset(state, 0), tokens);
    let account = entrance(account_card(state, tokens), card_offset(state, 1), tokens);
    let versions = entrance(version_card(state, tokens), card_offset(state, 2), tokens);

    let content = column![
        page_header("主页", "选择账户与版本，一键进入游戏", tokens),
        hero,
        row![account, versions]
            .spacing(theme::SPACE_LG)
            .width(Length::Fill),
    ]
    .spacing(theme::SPACE_LG)
    .padding(theme::SPACE_XL)
    .width(Length::Fill);

    scrollable(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 每帧推进：按压弹簧始终推进；入场未完成时累计时长、按 stagger 逐张触发并推进，全部落定后停摆。
pub fn tick(state: &mut State, dt: f32, _ctx: &Ctx) {
    state.launch_scale.step(dt);
    if state.enter_done {
        return;
    }
    state.enter_elapsed += dt;
    let elapsed = state.enter_elapsed;
    for (index, card) in state.cards.iter_mut().enumerate() {
        if elapsed >= index as f32 * ENTER_STAGGER {
            card.set(1.0);
        }
        card.step(dt);
    }
    if state.cards.iter().all(Animated::settled)
        && elapsed >= (CARD_COUNT as f32 - 1.0) * ENTER_STAGGER
    {
        state.enter_done = true;
    }
}

/// 是否仍有弹簧在动：入场未落定或按压未回弹时需继续挂帧。
pub fn animating(state: &State) -> bool {
    !state.enter_done || !state.launch_scale.settled()
}

// ---- 视图构件 ----

/// 卡片入场位移：随入场弹簧 0->1 由 [`ENTER_SHIFT`] 收敛到 0（过冲时短暂上越再落定）。
fn card_offset(state: &State, index: usize) -> f32 {
    (1.0 - state.cards[index].value()) * ENTER_SHIFT
}

/// 把内容包成玻璃卡并套入场竖向位移；外层再包一个 Fill 容器，保证在行内等分宽度。
fn entrance<'a>(
    content: Element<'a, Message>,
    offset_y: f32,
    tokens: Tokens,
) -> Element<'a, Message> {
    let card = glass_card(content, tokens).width(Length::Fill);
    let floated = float(card).translate(move |_content, _viewport| Vector::new(0.0, offset_y));
    container(floated).width(Length::Fill).into()
}

/// 快速启动 hero：标题 + 就绪摘要 +（可选加载错误）+ 大按钮 + 启动状态 + 刷新。
fn hero_card<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let on_press = state.ready().then_some(Message::Launch);
    let cta = primary_button("开始游戏", tokens, on_press).width(Length::Fill);
    let cta = spring_press(cta, state.launch_scale.value());
    let reload = secondary_button("刷新", tokens, (!state.loading).then_some(Message::Reload));

    let mut body = column![section_title("快速启动", tokens), hero_summary(state, tokens)]
        .spacing(theme::SPACE_MD);
    if let Some(error) = &state.error {
        body = body.push(
            text(format!("载入失败：{error}"))
                .size(theme::TEXT_BODY)
                .color(tokens.on_surface_muted),
        );
    }
    body.push(cta)
        .push(launch_status(state, tokens))
        .push(reload)
        .into()
}

/// 就绪摘要行：当前账户 · 当前版本（缺省给未选择提示）。
fn hero_summary<'a>(state: &State, tokens: Tokens) -> Element<'a, Message> {
    let account = state
        .account
        .as_ref()
        .map_or_else(|| "未选择账户".to_owned(), |a| format!("账户 {}", a.name));
    let version = state
        .selected
        .as_ref()
        .map_or_else(|| "未选择版本".to_owned(), |v| format!("版本 {v}"));
    row![
        text(account)
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted),
        text("·")
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted),
        text(version)
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted),
    ]
    .spacing(theme::SPACE_SM)
    .into()
}

/// 启动状态区文案。
fn launch_status<'a>(state: &State, tokens: Tokens) -> Element<'a, Message> {
    match &state.launch {
        LaunchStatus::Idle => {
            let hint = if state.ready() {
                "准备就绪，点击开始游戏"
            } else {
                "选择账户与版本后即可启动"
            };
            text(hint)
                .size(theme::TEXT_BODY)
                .color(tokens.on_surface_muted)
                .into()
        }
        LaunchStatus::Launching => loading("正在启动游戏…", tokens),
        LaunchStatus::Running(info) => {
            let pid = info
                .pid
                .map_or_else(|| "-".to_owned(), |pid| pid.to_string());
            text(format!("游戏已启动（版本 {}，PID {pid}）", info.version))
                .size(theme::TEXT_BODY)
                .color(tokens.on_surface)
                .into()
        }
        LaunchStatus::Failed(error) => text(format!("启动失败：{error}"))
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted)
            .into(),
    }
}

/// 当前账户卡：头像 + 名 + 类型；无账户时给引导（页面无法直接跳账户页，引导走左侧导航）。
fn account_card<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let body: Element<'a, Message> = match &state.account {
        Some(account) => row![
            avatar(&account.name, tokens),
            column![
                text(account.name.as_str())
                    .size(theme::TEXT_HEADING)
                    .color(tokens.on_surface),
                text(account_type_label(account.account_type))
                    .size(theme::TEXT_CAPTION)
                    .color(tokens.on_surface_muted),
            ]
            .spacing(theme::SPACE_XS),
        ]
        .spacing(theme::SPACE_MD)
        .align_y(Alignment::Center)
        .into(),
        None => column![
            row![
                icon(Icon::Account, 28.0, tokens.on_surface_muted),
                text("尚未选择账户")
                    .size(theme::TEXT_BODY)
                    .color(tokens.on_surface),
            ]
            .spacing(theme::SPACE_SM)
            .align_y(Alignment::Center),
            text("在左侧「账户」入口添加并选择账户")
                .size(theme::TEXT_CAPTION)
                .color(tokens.on_surface_muted),
        ]
        .spacing(theme::SPACE_SM)
        .into(),
    };

    column![section_title("当前账户", tokens), body]
        .spacing(theme::SPACE_MD)
        .into()
}

/// 圆形头像：强调渐变底 + 首字母。
fn avatar<'a>(name: &str, tokens: Tokens) -> Element<'a, Message> {
    let initial = name
        .chars()
        .next()
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().to_string());
    container(
        text(initial)
            .size(theme::TEXT_HEADING)
            .color(tokens.accent_text),
    )
    .width(Length::Fixed(AVATAR_SIZE))
    .height(Length::Fixed(AVATAR_SIZE))
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center)
    .style(move |_theme| container::Style {
        background: Some(Background::Gradient(tokens.accent_linear().into())),
        border: Border {
            radius: (AVATAR_SIZE / 2.0).into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// 账户类型的中文标签。
fn account_type_label(kind: AccountType) -> &'static str {
    match kind {
        AccountType::Microsoft => "微软正版",
        AccountType::Offline => "离线账户",
        AccountType::AuthlibInjector => "第三方验证",
    }
}

/// 版本选择卡：已安装版本逐行可选（选中态描边高亮）；无版本时给安装引导。
fn version_card<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let body: Element<'a, Message> = if state.versions.is_empty() {
        column![
            row![
                icon(Icon::Versions, 28.0, tokens.on_surface_muted),
                text("尚未安装任何版本")
                    .size(theme::TEXT_BODY)
                    .color(tokens.on_surface),
            ]
            .spacing(theme::SPACE_SM)
            .align_y(Alignment::Center),
            text("前往「版本」页安装一个版本后再启动")
                .size(theme::TEXT_CAPTION)
                .color(tokens.on_surface_muted),
        ]
        .spacing(theme::SPACE_SM)
        .into()
    } else {
        let mut list = column![].spacing(theme::SPACE_XS);
        for id in &state.versions {
            let selected = state.selected.as_deref() == Some(id.as_str());
            list = list.push(version_row(id, selected, tokens));
        }
        list.into()
    };

    column![section_title("版本选择", tokens), body]
        .spacing(theme::SPACE_MD)
        .into()
}

/// 单个版本可选行：细线版本图标 + id；选中态填选中色并描强调边，悬停填弱高光。
fn version_row<'a>(id: &'a str, selected: bool, tokens: Tokens) -> Element<'a, Message> {
    let content = row![
        icon(Icon::Versions, 16.0, tokens.icon),
        text(id).size(theme::TEXT_BODY).color(tokens.on_surface),
    ]
    .spacing(theme::SPACE_SM)
    .align_y(Alignment::Center);

    button(content)
        .width(Length::Fill)
        .padding([theme::SPACE_SM, theme::SPACE_MD])
        .on_press(Message::SelectVersion(id.to_owned()))
        .style(move |_theme, status| {
            let background = if selected {
                Some(Background::Color(tokens.selected))
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                Some(Background::Color(tokens.hover))
            } else {
                None
            };
            button::Style {
                background,
                text_color: tokens.on_surface,
                border: Border {
                    color: if selected {
                        tokens.accent_from
                    } else {
                        Color::TRANSPARENT
                    },
                    width: if selected { 1.0 } else { 0.0 },
                    radius: theme::RADIUS_SM.into(),
                },
                ..button::Style::default()
            }
        })
        .into()
}
