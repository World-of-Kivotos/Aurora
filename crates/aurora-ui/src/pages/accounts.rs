//! 账户页：多账户管理（微软正版 + 离线身份）。
//!
//! 功能：
//! 1. 账户列表——头像 + 名称 + 类型 +「当前」标记，支持切换当前账户与移除账户。
//! 2. 添加账户——两个标签页：
//!    - 微软正版：走 OAuth 设备码流。`core.microsoft_login(on_code)` 把整条链（申请设备码 -> 轮询 ->
//!      令牌链 -> 落库）打包成一次异步调用，`on_code` 同步回调在拿到设备码时触发。为把「先展示设备码、
//!      后台继续轮询」拆成 UI 可消费的两拍，用 `iced::stream::channel` 建一条流：`on_code` 里 `try_send`
//!      设备码（非阻塞），链结束后 `send` 最终结果，`Task::run` 消费该流回本页 `Message::Login`。
//!    - 离线身份：输入用户名，`core.create_offline_account` 做合法性校验并生成稳定离线 UUID。
//!
//! 后端能力缺口（如实记录，未伪造接口）：
//! - `create_offline_account` 只做「即用即弃」的离线身份，门面没有把离线账户写入账户库的入口
//!   （`AccountManager::upsert` 仅 `microsoft_login` 内部可达）。故离线身份保存在本页内存中，标注「仅本
//!   次会话」，其「设为当前」是本地选择；持久化的当前账户仅覆盖微软账户（`set_current_account`）。
//! - 门面无 `&self` 的 client_id 注入口（`set_client_id` 需 `&mut`，经 `Arc` 不可达）。故调试 client_id
//!   经 `core` 自身认可的环境变量回落 `AURORA_MSA_CLIENT_ID` 注入（见 aurora-core auth 模块）。

use std::sync::Arc;

use aurora_core::{Account, AccountType, Aurora, DeviceCodeResponse, MSA_CLIENT_ID_ENV};
use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, Stream};
use iced::widget::{column, container, row, scrollable, text, text_input};
use iced::{Alignment, Background, Border, Element, Fill, Length, Task};

use crate::anim::{self, Animated};
use crate::ctx::Ctx;
use crate::theme::{self, Tokens};
use crate::widgets::{
    Icon, empty_state, glass_card, icon, loading, page_header, primary_button, secondary_button,
    section_title, sliding_card, spring_press,
};

/// 任务指定的调试用微软登录 client_id。仅在 config 与环境变量都缺失时作为回落注入。
const DEBUG_MSA_CLIENT_ID: &str = "00000000402B5328";

/// 账户卡入场初始横向位移（px），弹簧收敛到 0。
const CARD_SLIDE: f32 = 20.0;
/// 相邻账户卡入场的错峰延迟（秒），形成 stagger。
const CARD_STAGGER: f32 = 0.06;
/// 按压弹性的起始缩放（从此值弹回 1.0，形成「按下回弹」触感）。
const PRESS_FROM: f32 = 0.94;

/// 账户来源：后端持久化（微软账户，可经门面切换/删除）或本会话离线身份（内存态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Persisted,
    Session,
}

/// 列表展示用的账户摘要（Message 需 Clone，故只留可廉价克隆的字段）。
#[derive(Debug, Clone)]
struct AccountView {
    uuid: String,
    name: String,
    kind: AccountType,
    origin: Origin,
}

impl AccountView {
    /// 由后端持久化账户构造。
    fn persisted(account: &Account) -> Self {
        Self {
            uuid: account.uuid.clone(),
            name: account.name.clone(),
            kind: account.account_type,
            origin: Origin::Persisted,
        }
    }

    /// 由本会话离线身份构造。
    fn session(account: &Account) -> Self {
        Self {
            uuid: account.uuid.clone(),
            name: account.name.clone(),
            kind: account.account_type,
            origin: Origin::Session,
        }
    }
}

/// 一次列表读取的结果：账户列表 + 后端当前账户 uuid。
#[derive(Debug, Clone)]
pub(crate) struct AccountList {
    accounts: Vec<AccountView>,
    current: Option<String>,
}

/// 设备码流展示所需字段（面向用户）。
#[derive(Debug, Clone)]
pub struct DeviceCode {
    user_code: String,
    verification_uri: String,
}

impl From<&DeviceCodeResponse> for DeviceCode {
    fn from(response: &DeviceCodeResponse) -> Self {
        Self {
            user_code: response.user_code.clone(),
            verification_uri: response.verification_uri.clone(),
        }
    }
}

/// 微软登录流的两拍事件：先出设备码，后出最终结果。
#[derive(Debug, Clone)]
pub enum LoginEvent {
    /// 设备码就绪，供 UI 展示；后台继续轮询。
    Code(DeviceCode),
    /// 登录链完成：成功（账户已由 core 落库）或失败文案。
    Done(Result<(), String>),
}

/// 微软登录的界面状态。
enum LoginState {
    Idle,
    /// 已发起，尚未拿到设备码。
    Awaiting,
    /// 设备码已展示，后台轮询中。
    Pending(DeviceCode),
    /// 失败（展示原因，可重试）。
    Failed(String),
}

/// 添加面板的标签页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddTab {
    Microsoft,
    Offline,
}

/// 按压弹性的作用目标（同一时刻只弹一个）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum PressTarget {
    None,
    /// 添加面板的主操作按钮。
    Cta,
    /// 某张账户卡（按 uuid 定位）。
    Card(String),
}

/// 单张账户卡的入场动画：延迟释放后，横向位移弹簧收敛到 0。
struct CardAnim {
    offset: Animated,
    delay: f32,
    released: bool,
}

impl CardAnim {
    /// 待入场：位移停在 [`CARD_SLIDE`]，延迟 `delay` 秒后释放（设目标 0）。
    fn entering(index: usize) -> Self {
        Self {
            offset: Animated::new(CARD_SLIDE, anim::aurora_enter()),
            delay: index as f32 * CARD_STAGGER,
            released: false,
        }
    }

    /// 静止：已在位（无入场动画，用于切换当前账户等不重排的场景）。
    fn rest() -> Self {
        Self {
            offset: Animated::new(0.0, anim::aurora_enter()),
            delay: 0.0,
            released: true,
        }
    }
}

/// 页面状态。
pub struct State {
    /// 后端持久化账户（微软）。
    persisted: Vec<AccountView>,
    /// 本会话离线身份（未持久化）。
    offline_items: Vec<AccountView>,
    /// 后端当前账户 uuid。
    backend_current: Option<String>,
    /// 本地选中的离线身份 uuid（优先于后端当前作为「当前」显示）。
    selected_offline: Option<String>,
    /// 列表加载中。
    loading: bool,
    /// 顶层错误（列表加载 / 切换 / 删除失败）。
    error: Option<String>,
    /// 添加面板当前标签页。
    add_tab: AddTab,
    /// 离线用户名输入。
    offline_name: String,
    /// 离线用户名的实时校验错误。
    offline_error: Option<String>,
    /// 微软登录状态。
    login: LoginState,
    /// 微软登录后台任务句柄（drop 即中止轮询）。
    login_handle: Option<iced::task::Handle>,
    /// 与展示列表（persisted ++ offline_items）并行的入场动画。
    anims: Vec<CardAnim>,
    /// 按压弹性缩放。
    press: Animated,
    /// 按压弹性作用目标。
    press_target: PressTarget,
}

impl Default for State {
    fn default() -> Self {
        Self {
            persisted: Vec::new(),
            offline_items: Vec::new(),
            backend_current: None,
            selected_offline: None,
            loading: false,
            error: None,
            add_tab: AddTab::Microsoft,
            offline_name: String::new(),
            offline_error: None,
            login: LoginState::Idle,
            login_handle: None,
            anims: Vec::new(),
            press: Animated::new(1.0, anim::aurora_press()),
            press_target: PressTarget::None,
        }
    }
}

/// 页面消息。
#[derive(Debug, Clone)]
pub enum Message {
    /// 列表加载完成（带入场 stagger：初次进入 / 移除 / 登录后）。
    Loaded(Result<AccountList, String>),
    /// 切换当前账户后的列表更新（不重排）。
    CurrentChanged(Result<AccountList, String>),
    /// 切换添加面板标签页。
    SelectTab(AddTab),
    /// 离线用户名输入变化。
    OfflineNameChanged(String),
    /// 创建离线身份。
    CreateOffline,
    /// 发起微软登录。
    StartMicrosoft,
    /// 取消微软登录（中止后台轮询）。
    CancelMicrosoft,
    /// 微软登录流事件。
    Login(LoginEvent),
    /// 将某持久化账户设为当前。
    SetCurrent(String),
    /// 选中某离线身份为当前（本地）。
    UseOffline(String),
    /// 移除某持久化账户。
    RemovePersisted(String),
    /// 移除某离线身份（本地）。
    RemoveOffline(String),
}

/// 构造页面状态与首个副作用：进入即读取本地账户库（DPAPI 解密，开销小），填充列表。
pub fn init(ctx: &Ctx) -> (State, Task<Message>) {
    let state = State {
        loading: true,
        ..State::default()
    };
    let core = ctx.core.clone();
    (state, Task::perform(load_accounts(core), Message::Loaded))
}

/// 处理页面消息。
pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Loaded(result) => {
            state.loading = false;
            match result {
                Ok(list) => {
                    state.persisted = list.accounts;
                    state.backend_current = list.current;
                    state.error = None;
                    rebuild_anims(state, true);
                }
                Err(error) => state.error = Some(error),
            }
            Task::none()
        }
        Message::CurrentChanged(result) => {
            state.loading = false;
            match result {
                Ok(list) => {
                    state.persisted = list.accounts;
                    state.backend_current = list.current;
                    state.error = None;
                    ensure_anims(state);
                }
                Err(error) => state.error = Some(error),
            }
            Task::none()
        }
        Message::SelectTab(tab) => {
            state.add_tab = tab;
            Task::none()
        }
        Message::OfflineNameChanged(name) => {
            state.offline_name = name;
            state.offline_error = validate_offline(state, ctx);
            Task::none()
        }
        Message::CreateOffline => {
            pop(state, PressTarget::Cta);
            let name = state.offline_name.trim().to_owned();
            // 复用后端校验与稳定 UUID 生成（不落库，见模块头缺口说明）。
            match ctx.core.create_offline_account(&name, None) {
                Ok(account) => {
                    let exists = state.offline_items.iter().any(|a| a.uuid == account.uuid)
                        || state.persisted.iter().any(|a| a.uuid == account.uuid);
                    if exists {
                        state.offline_error = Some("该离线身份已存在".to_owned());
                    } else {
                        state.offline_items.push(AccountView::session(&account));
                        state.offline_name.clear();
                        state.offline_error = None;
                        rebuild_anims(state, true);
                    }
                }
                Err(error) => state.offline_error = Some(error.to_string()),
            }
            Task::none()
        }
        Message::StartMicrosoft => {
            pop(state, PressTarget::Cta);
            ensure_debug_client_id(&ctx.core);
            state.login = LoginState::Awaiting;
            let core = ctx.core.clone();
            let (task, handle) =
                Task::run(microsoft_login_stream(core), Message::Login).abortable();
            // abort_on_drop：State 释放或被新登录覆盖时自动中止后台轮询。
            state.login_handle = Some(handle.abort_on_drop());
            task
        }
        Message::CancelMicrosoft => {
            state.login = LoginState::Idle;
            state.login_handle = None;
            Task::none()
        }
        Message::Login(event) => match event {
            LoginEvent::Code(code) => {
                // 用户可能已取消，仅在仍处流程中才展示设备码。
                if matches!(state.login, LoginState::Awaiting | LoginState::Pending(_)) {
                    state.login = LoginState::Pending(code);
                }
                Task::none()
            }
            LoginEvent::Done(Ok(())) => {
                state.login = LoginState::Idle;
                state.login_handle = None;
                // 账户已落库，重载持久化列表（带入场 stagger）。
                state.loading = true;
                let core = ctx.core.clone();
                Task::perform(load_accounts(core), Message::Loaded)
            }
            LoginEvent::Done(Err(error)) => {
                if matches!(state.login, LoginState::Awaiting | LoginState::Pending(_)) {
                    state.login = LoginState::Failed(error);
                }
                state.login_handle = None;
                Task::none()
            }
        },
        Message::SetCurrent(uuid) => {
            pop(state, PressTarget::Card(uuid.clone()));
            state.selected_offline = None;
            state.loading = true;
            let core = ctx.core.clone();
            Task::perform(set_current_then_load(core, uuid), Message::CurrentChanged)
        }
        Message::UseOffline(uuid) => {
            pop(state, PressTarget::Card(uuid.clone()));
            state.selected_offline = Some(uuid);
            Task::none()
        }
        Message::RemovePersisted(uuid) => {
            state.loading = true;
            let core = ctx.core.clone();
            Task::perform(remove_then_load(core, uuid), Message::Loaded)
        }
        Message::RemoveOffline(uuid) => {
            state.offline_items.retain(|a| a.uuid != uuid);
            if state.selected_offline.as_deref() == Some(uuid.as_str()) {
                state.selected_offline = None;
            }
            rebuild_anims(state, true);
            Task::none()
        }
    }
}

/// 渲染页面。
pub fn view<'a>(state: &'a State, ctx: &Ctx) -> Element<'a, Message> {
    let tokens = ctx.tokens();

    let mut page = column![page_header("账户", "管理离线与微软正版账户", tokens)]
        .spacing(theme::SPACE_LG)
        .padding(theme::SPACE_XL)
        .width(Fill);

    if let Some(error) = &state.error {
        page = page.push(
            text(format!("操作失败：{error}"))
                .size(theme::TEXT_BODY)
                .color(tokens.accent_to),
        );
    }

    page = page.push(add_card(state, tokens));
    page = page.push(list_section(state, tokens));

    scrollable(page).width(Fill).height(Fill).into()
}

/// 添加账户卡：标签页切换 + 对应面板。
fn add_card<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let tabs = row![
        tab_button(
            "微软正版",
            state.add_tab == AddTab::Microsoft,
            Message::SelectTab(AddTab::Microsoft),
            tokens,
        ),
        tab_button(
            "离线身份",
            state.add_tab == AddTab::Offline,
            Message::SelectTab(AddTab::Offline),
            tokens,
        ),
    ]
    .spacing(theme::SPACE_SM);

    let body = match state.add_tab {
        AddTab::Microsoft => microsoft_panel(state, tokens),
        AddTab::Offline => offline_panel(state, tokens),
    };

    glass_card(
        column![section_title("添加账户", tokens), tabs, body]
            .spacing(theme::SPACE_MD)
            .width(Fill),
        tokens,
    )
    .width(Fill)
    .into()
}

/// 标签按钮：当前标签用主按钮实底，其余用次按钮描边。
fn tab_button<'a>(
    label: &'a str,
    active: bool,
    message: Message,
    tokens: Tokens,
) -> iced::widget::button::Button<'a, Message> {
    if active {
        primary_button(label, tokens, Some(message))
    } else {
        secondary_button(label, tokens, Some(message))
    }
}

/// 微软登录面板：按登录状态切换内容。
fn microsoft_panel<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let intro = text("使用微软账户登录，获取正版游戏授权。")
        .size(theme::TEXT_BODY)
        .color(tokens.on_surface_muted);

    match &state.login {
        LoginState::Idle => column![
            intro,
            spring_press(
                primary_button("微软登录", tokens, Some(Message::StartMicrosoft)),
                cta_scale(state),
            ),
        ]
        .spacing(theme::SPACE_MD)
        .into(),
        LoginState::Failed(error) => column![
            intro,
            text(format!("登录失败：{error}"))
                .size(theme::TEXT_BODY)
                .color(tokens.accent_to),
            spring_press(
                primary_button("重新登录", tokens, Some(Message::StartMicrosoft)),
                cta_scale(state),
            ),
        ]
        .spacing(theme::SPACE_MD)
        .into(),
        LoginState::Awaiting => column![
            loading("正在向微软申请设备码…", tokens),
            secondary_button("取消", tokens, Some(Message::CancelMicrosoft)),
        ]
        .spacing(theme::SPACE_MD)
        .into(),
        LoginState::Pending(code) => device_code_panel(code, tokens),
    }
}

/// 设备码展示：验证网址 + 大号验证码 + 轮询提示 + 取消。
fn device_code_panel<'a>(code: &'a DeviceCode, tokens: Tokens) -> Element<'a, Message> {
    let code_box = container(
        text(code.user_code.as_str())
            .size(theme::TEXT_TITLE)
            .color(tokens.on_surface),
    )
    .padding([theme::SPACE_SM, theme::SPACE_LG])
    .style(move |_theme| container::Style {
        background: Some(Background::Color(tokens.surface)),
        border: Border {
            color: tokens.accent_from,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..container::Style::default()
    });

    column![
        text("请在浏览器打开以下网址并输入验证码：")
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface),
        text(code.verification_uri.as_str())
            .size(theme::TEXT_BODY)
            .color(tokens.accent_from),
        code_box,
        loading("完成网页登录后，应用会自动检测并登录…", tokens),
        secondary_button("取消", tokens, Some(Message::CancelMicrosoft)),
    ]
    .spacing(theme::SPACE_MD)
    .into()
}

/// 离线身份面板：用户名输入 + 实时校验 + 创建。
fn offline_panel<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let input = text_input("用户名，如 Steve", state.offline_name.as_str())
        .on_input(Message::OfflineNameChanged)
        .on_submit(Message::CreateOffline)
        .padding([theme::SPACE_SM, theme::SPACE_MD])
        .size(theme::TEXT_BODY)
        .width(Fill)
        .style(input_style(tokens));

    let can_create = !state.offline_name.trim().is_empty() && state.offline_error.is_none();
    let submit = spring_press(
        primary_button(
            "创建离线身份",
            tokens,
            can_create.then_some(Message::CreateOffline),
        ),
        cta_scale(state),
    );

    let mut col = column![
        text("输入游戏内用户名，创建离线身份用于离线启动（仅本次会话）。")
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted),
        input,
    ]
    .spacing(theme::SPACE_MD);

    if let Some(error) = &state.offline_error {
        col = col.push(
            text(error.as_str())
                .size(theme::TEXT_CAPTION)
                .color(tokens.accent_to),
        );
    }

    col.push(submit).into()
}

/// 账户列表区：加载中 / 空态 / 列表。
fn list_section<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let empty = state.persisted.is_empty() && state.offline_items.is_empty();

    if state.loading && empty {
        return glass_card(loading("正在读取账户…", tokens), tokens)
            .width(Fill)
            .into();
    }

    if empty {
        return empty_state(
            Icon::Account,
            "尚未添加账户",
            "使用上方的微软登录或离线身份添加一个账户",
            tokens,
        );
    }

    column![section_title("账户列表", tokens), account_list(state, tokens)]
        .spacing(theme::SPACE_MD)
        .width(Fill)
        .into()
}

/// 账户列表：持久化账户在前，离线身份在后；逐张套入场位移。
fn account_list<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let current = effective_current(state);
    let mut list = column![].spacing(theme::SPACE_MD).width(Fill);

    for (index, account) in state
        .persisted
        .iter()
        .chain(state.offline_items.iter())
        .enumerate()
    {
        let offset = state.anims.get(index).map_or(0.0, |a| a.offset.value());
        let is_current = current == Some(account.uuid.as_str());
        list = list.push(account_card(state, account, is_current, offset, tokens));
    }

    list.into()
}

/// 单张账户卡：头像 + 名称/类型 + 「当前」标记或「设为当前」+ 「移除」。
fn account_card<'a>(
    state: &'a State,
    account: &'a AccountView,
    is_current: bool,
    offset: f32,
    tokens: Tokens,
) -> Element<'a, Message> {
    let mut info = column![
        text(account.name.as_str())
            .size(theme::TEXT_HEADING)
            .color(tokens.on_surface),
        text(kind_label(account.kind))
            .size(theme::TEXT_CAPTION)
            .color(tokens.on_surface_muted),
    ]
    .spacing(theme::SPACE_XS)
    .width(Fill);

    if matches!(account.origin, Origin::Session) {
        info = info.push(
            text("仅本次会话可用，未持久化")
                .size(theme::TEXT_CAPTION)
                .color(tokens.on_surface_muted),
        );
    }

    let mut actions = row![].spacing(theme::SPACE_SM).align_y(Alignment::Center);
    if is_current {
        actions = actions.push(current_badge(tokens));
    } else {
        let set_message = match account.origin {
            Origin::Persisted => Message::SetCurrent(account.uuid.clone()),
            Origin::Session => Message::UseOffline(account.uuid.clone()),
        };
        actions = actions.push(secondary_button("设为当前", tokens, Some(set_message)));
    }
    let remove_message = match account.origin {
        Origin::Persisted => Message::RemovePersisted(account.uuid.clone()),
        Origin::Session => Message::RemoveOffline(account.uuid.clone()),
    };
    actions = actions.push(secondary_button("移除", tokens, Some(remove_message)));

    let content = row![avatar(tokens), info, actions]
        .spacing(theme::SPACE_MD)
        .align_y(Alignment::Center);

    let card = sliding_card(content, (offset, 0.0), tokens);

    // 按压弹性只作用于被操作的那张卡。
    if state.press_target == PressTarget::Card(account.uuid.clone()) && !state.press.settled() {
        spring_press(card, state.press.value())
    } else {
        card
    }
}

/// 圆形头像：强调渐变底 + 白色人像描边图标。
fn avatar<'a>(tokens: Tokens) -> Element<'a, Message> {
    container(icon(Icon::Account, 22.0, tokens.accent_text))
        .center_x(Length::Fixed(44.0))
        .center_y(Length::Fixed(44.0))
        .style(move |_theme| container::Style {
            background: Some(Background::Gradient(tokens.accent_linear().into())),
            border: Border {
                radius: 22.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// 「当前」标记：强调渐变小药丸。
fn current_badge<'a>(tokens: Tokens) -> Element<'a, Message> {
    container(
        text("当前")
            .size(theme::TEXT_CAPTION)
            .color(tokens.accent_text),
    )
    .padding([theme::SPACE_XS, theme::SPACE_SM])
    .style(move |_theme| container::Style {
        background: Some(Background::Gradient(tokens.accent_linear().into())),
        border: Border {
            radius: theme::RADIUS_SM.into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// 文本输入样式：玻璃面 + 聚焦时强调色描边。
fn input_style(tokens: Tokens) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style {
    move |_theme, status| {
        let focused = matches!(status, text_input::Status::Focused { .. });
        text_input::Style {
            background: Background::Color(tokens.surface),
            border: Border {
                color: if focused {
                    tokens.accent_from
                } else {
                    tokens.surface_border
                },
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            icon: tokens.on_surface_muted,
            placeholder: tokens.on_surface_muted,
            value: tokens.on_surface,
            selection: tokens.selected,
        }
    }
}

/// 账户类型中文标签。
fn kind_label(kind: AccountType) -> &'static str {
    match kind {
        AccountType::Microsoft => "微软正版",
        AccountType::Offline => "离线",
        AccountType::AuthlibInjector => "外置登录",
    }
}

/// 生效的「当前账户」uuid：本地选中的离线身份优先，否则后端当前账户。
fn effective_current(state: &State) -> Option<&str> {
    state
        .selected_offline
        .as_deref()
        .or(state.backend_current.as_deref())
}

/// CTA 按钮的按压缩放：仅当作用目标为 CTA 且弹簧未收敛时生效。
fn cta_scale(state: &State) -> f32 {
    if state.press_target == PressTarget::Cta && !state.press.settled() {
        state.press.value()
    } else {
        1.0
    }
}

/// 触发一次按压弹性：从 [`PRESS_FROM`] 弹回 1.0，作用于给定目标。
fn pop(state: &mut State, target: PressTarget) {
    state.press = Animated::new(PRESS_FROM, anim::aurora_press());
    state.press.set(1.0);
    state.press_target = target;
}

/// 重建入场动画表，长度对齐展示列表。`animate` 为 true 时逐张错峰入场，否则直接静止在位。
fn rebuild_anims(state: &mut State, animate: bool) {
    let count = state.persisted.len() + state.offline_items.len();
    state.anims = (0..count)
        .map(|index| {
            if animate {
                CardAnim::entering(index)
            } else {
                CardAnim::rest()
            }
        })
        .collect();
}

/// 保证动画表长度与展示列表一致（用于切换当前账户等不重排场景）；长度变化才补齐为静止。
fn ensure_anims(state: &mut State) {
    let count = state.persisted.len() + state.offline_items.len();
    if state.anims.len() != count {
        state.anims = (0..count).map(|_| CardAnim::rest()).collect();
    }
}

/// 实时校验离线用户名，复用后端规则（`create_offline_account` 内含校验）。空串不报错（仅禁用按钮）。
fn validate_offline(state: &State, ctx: &Ctx) -> Option<String> {
    let name = state.offline_name.trim();
    if name.is_empty() {
        return None;
    }
    match ctx.core.create_offline_account(name, None) {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    }
}

/// 保证调试 client_id 可用：config 与环境变量都缺失时，注入调试回落值。
fn ensure_debug_client_id(core: &Aurora) {
    let configured = core
        .config()
        .msa_client_id
        .as_deref()
        .is_some_and(|value| !value.is_empty());
    let from_env = std::env::var(MSA_CLIENT_ID_ENV).is_ok_and(|value| !value.is_empty());
    if !configured && !from_env {
        // 门面无 &self 的 client_id 注入口（set_client_id 需 &mut，经 Arc 不可达），而 core 的
        // msa_client_id() 会把此环境变量当作调试回落来源，故按其契约注入。
        // SAFETY: 仅在 UI 更新循环（主线程）内、变量缺失时设置一次；后台登录任务此刻尚未启动读取。
        unsafe {
            std::env::set_var(MSA_CLIENT_ID_ENV, DEBUG_MSA_CLIENT_ID);
        }
    }
}

/// 读取账户库，映射成小摘要。`accounts()`/`current_account()` 为本地文件（DPAPI）读取，开销小。
fn read_accounts(core: &Aurora) -> Result<AccountList, String> {
    let accounts = core.accounts().map_err(|error| error.to_string())?;
    let current = core
        .current_account()
        .map_err(|error| error.to_string())?
        .map(|account| account.uuid);
    Ok(AccountList {
        accounts: accounts.iter().map(AccountView::persisted).collect(),
        current,
    })
}

/// 异步读取账户列表（经 `Arc<Aurora>` 调 `&self` 方法）。
async fn load_accounts(core: Arc<Aurora>) -> Result<AccountList, String> {
    read_accounts(&core)
}

/// 切换当前账户后重读列表。
async fn set_current_then_load(core: Arc<Aurora>, uuid: String) -> Result<AccountList, String> {
    core.set_current_account(&uuid)
        .map_err(|error| error.to_string())?;
    read_accounts(&core)
}

/// 移除账户后重读列表。
async fn remove_then_load(core: Arc<Aurora>, uuid: String) -> Result<AccountList, String> {
    core.remove_account(&uuid)
        .map_err(|error| error.to_string())?;
    read_accounts(&core)
}

/// 微软设备码登录流：把「设备码 -> 结果」两拍事件汇入一条流供 `Task::run` 消费。
///
/// `on_code` 是同步回调，在链路拿到设备码时触发；`try_send` 非阻塞地把设备码送回 UI（缓冲区足够，不会
/// 因通道满而丢弃首拍）。链路结束后再 `send` 最终结果。两个发送端在闭包结束时释放，接收端随之收束，流终止。
fn microsoft_login_stream(core: Arc<Aurora>) -> impl Stream<Item = LoginEvent> {
    iced::stream::channel(4, async move |mut output: mpsc::Sender<LoginEvent>| {
        let mut code_tx = output.clone();
        let result = core
            .microsoft_login(move |device: &DeviceCodeResponse| {
                let _ = code_tx.try_send(LoginEvent::Code(DeviceCode::from(device)));
            })
            .await;
        // 成功只回信号（账户已由 core 落库，随后 update 触发列表重载）；失败在边界转字符串。
        let outcome = result.map(|_account| ()).map_err(|error| error.to_string());
        let _ = output.send(LoginEvent::Done(outcome)).await;
    })
}

/// 每帧推进：释放到期的入场卡、步进各弹簧。
pub fn tick(state: &mut State, dt: f32, _ctx: &Ctx) {
    for card in &mut state.anims {
        if !card.released {
            card.delay -= dt;
            if card.delay <= 0.0 {
                card.offset.set(0.0);
                card.released = true;
            }
        }
        card.offset.step(dt);
    }
    state.press.step(dt);
}

/// 是否仍有动画未收敛：按压弹簧未静止，或有卡片尚未释放 / 位移未收敛。
pub fn animating(state: &State) -> bool {
    !state.press.settled()
        || state
            .anims
            .iter()
            .any(|card| !card.released || !card.offset.settled())
}
