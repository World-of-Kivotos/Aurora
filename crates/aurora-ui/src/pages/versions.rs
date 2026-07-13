//! 版本页：已安装版本列表 + 安装新版本（抓清单、筛选、选加载器、触发安装、进度反馈）。
//!
//! 后端调用一律经 `ctx.core`（`Arc<Aurora>`）+ `Task::perform` 异步范式：克隆句柄进异步块，回本页
//! `Message`（外壳再 `.map` 回全局），错误在边界转 `String`（`CoreError` 不 `Clone`）。
//!
//! 动效：已安装卡片入场按索引错峰弹入（各持一枚 `anim::Animated` 的 y 位移弹簧）；安装按钮点按弹性
//! 缩放；安装进行时以一条循环扫动的强调色进度条表达「工作中」。`animating` 覆盖这三处未收敛的弹簧
//! 以及安装进行态，收敛即停帧。
//!
//! 后端能力边界（见 aurora-core）：`install` 目前只支持 `LoaderChoice::{Fabric, Quilt}`，Forge/NeoForge
//! 需本地安装器执行 processors，尚未接入，故加载器仅提供 原版 / Fabric / Quilt 三档，界面明示其余待接入。
//! 逐字节下载进度需向 `install` 传入 `EventSink`（`tokio::sync::mpsc` 发送端），而本 crate 未直接依赖
//! tokio、无法构造该通道，故本页以不定量扫动进度替代；接入真实进度需另行补齐依赖或由 core 暴露流式接口。

use std::sync::Arc;

use aurora_core::{Aurora, LoaderChoice};
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Background, Border, Element, Length, Task};

use crate::anim::{self, Animated};
use crate::ctx::Ctx;
use crate::theme::{self, Tokens};
use crate::widgets::{
    Icon, empty_state, glass_card, icon, loading, page_header, primary_button, progress,
    secondary_button, section_title, sliding_card, spring_press,
};

/// 已安装卡片入场 y 位移初值（像素，向下偏移后弹回 0）。
const ENTER_OFFSET: f32 = 18.0;
/// 相邻卡片入场错峰步长（秒）。
const STAGGER_STEP: f32 = 0.06;
/// 错峰延迟的索引上限，超出的卡片一起入场，避免长列表拖出过长的入场尾巴。
const MAX_STAGGER: usize = 10;
/// 安装按钮点按初始缩放（弹回 1.0 形成脆弹）。
const PRESS_SCALE: f32 = 0.9;
/// 安装进行时扫动进度的单程时长（秒）。
const SWEEP_DURATION: f32 = 0.9;
/// 版本清单单次最多渲染的行数（iced 无虚拟化，过多行会拖慢布局；用筛选/搜索缩小范围）。
const MAX_ROWS: usize = 200;

/// 已安装版本的展示摘要（Message 需 Clone，只留可廉价克隆的字段）。
#[derive(Debug, Clone)]
struct InstalledVersion {
    /// 版本 id（等于版本目录名）。
    id: String,
    /// 加载器标签（如 `Fabric 0.15.11`）；原版为 `None`。
    loader: Option<String>,
    /// 是否正式版。
    is_release: bool,
}

/// 一次已安装扫描的展示结果。
#[derive(Debug, Clone)]
pub struct InstalledScan {
    versions: Vec<InstalledVersion>,
    /// 无法解析的损坏版本目录数（成列展示留待后续，先给计数提示）。
    broken: usize,
}

/// 版本清单中的可安装条目摘要。
#[derive(Debug, Clone)]
struct ManifestEntry {
    id: String,
    is_release: bool,
    is_snapshot: bool,
}

/// 一次清单抓取的展示结果。
#[derive(Debug, Clone)]
pub struct ManifestData {
    versions: Vec<ManifestEntry>,
    latest_release: String,
    latest_snapshot: String,
}

/// 清单筛选档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionFilter {
    Release,
    Snapshot,
    All,
}

/// 加载器选择（映射到后端 `Option<LoaderChoice>`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderPick {
    Vanilla,
    Fabric,
    Quilt,
}

impl LoaderPick {
    /// 映射到后端安装参数。原版为 `None`。
    fn choice(self) -> Option<LoaderChoice> {
        match self {
            LoaderPick::Vanilla => None,
            LoaderPick::Fabric => Some(LoaderChoice::Fabric),
            LoaderPick::Quilt => Some(LoaderChoice::Quilt),
        }
    }

    /// 目标文案后缀（拼在版本 id 之后）。
    fn suffix(self) -> &'static str {
        match self {
            LoaderPick::Vanilla => "",
            LoaderPick::Fabric => " + Fabric",
            LoaderPick::Quilt => " + Quilt",
        }
    }
}

/// 单枚已安装卡片的入场弹簧：先按 `delay` 计时，到点后把 y 位移目标设 0 弹回。
#[derive(Debug, Clone, Copy)]
struct CardEntrance {
    offset: Animated,
    delay: f32,
    released: bool,
}

/// 页面状态。
#[derive(Debug)]
pub struct State {
    // 已安装
    installed: Vec<InstalledVersion>,
    installed_loading: bool,
    installed_error: Option<String>,
    broken_count: usize,
    cards: Vec<CardEntrance>,

    // 版本清单（安装源）
    manifest: Vec<ManifestEntry>,
    latest_release: Option<String>,
    latest_snapshot: Option<String>,
    manifest_loading: bool,
    manifest_error: Option<String>,
    manifest_loaded: bool,

    // 安装选择
    filter: VersionFilter,
    search: String,
    selected_version: Option<String>,
    selected_loader: LoaderPick,

    // 安装进行态
    installing: bool,
    install_label: String,
    install_error: Option<String>,
    install_done: Option<String>,
    sweep: Animated,
    install_pop: Animated,
}

impl Default for State {
    fn default() -> Self {
        Self {
            installed: Vec::new(),
            // init 进入即发起一次本地扫描，默认呈加载态。
            installed_loading: true,
            installed_error: None,
            broken_count: 0,
            cards: Vec::new(),
            manifest: Vec::new(),
            latest_release: None,
            latest_snapshot: None,
            manifest_loading: false,
            manifest_error: None,
            manifest_loaded: false,
            filter: VersionFilter::Release,
            search: String::new(),
            selected_version: None,
            selected_loader: LoaderPick::Vanilla,
            installing: false,
            install_label: String::new(),
            install_error: None,
            install_done: None,
            sweep: Animated::new(0.0, anim::aurora()),
            install_pop: Animated::new(1.0, anim::aurora_press()),
        }
    }
}

/// 页面消息。
#[derive(Debug, Clone)]
pub enum Message {
    /// 重新扫描本地已安装版本。
    RefreshInstalled,
    /// 已安装扫描完成。
    InstalledLoaded(Result<InstalledScan, String>),
    /// 抓取远端版本清单。
    FetchManifest,
    /// 清单抓取完成。
    ManifestLoaded(Result<ManifestData, String>),
    /// 切换筛选档位。
    FilterChanged(VersionFilter),
    /// 搜索框输入。
    SearchChanged(String),
    /// 选中一个待安装版本。
    SelectVersion(String),
    /// 切换加载器选择。
    LoaderChanged(LoaderPick),
    /// 触发安装所选版本。
    Install,
    /// 安装完成（成功摘要或错误说明）。
    Installed(Result<String, String>),
}

/// 进入应用即扫描本地已安装版本（离线、廉价）；远端清单交给用户按需抓取，避免每次切页都联网。
pub fn init(ctx: &Ctx) -> (State, Task<Message>) {
    let core = ctx.core.clone();
    (
        State::default(),
        Task::perform(scan_installed(core), Message::InstalledLoaded),
    )
}

/// 处理页面消息。
pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::RefreshInstalled => {
            state.installed_loading = true;
            state.installed_error = None;
            Task::perform(scan_installed(ctx.core.clone()), Message::InstalledLoaded)
        }
        Message::InstalledLoaded(result) => {
            state.installed_loading = false;
            match result {
                Ok(scan) => {
                    state.broken_count = scan.broken;
                    state.installed = scan.versions;
                    let count = state.installed.len();
                    state.cards = (0..count).map(build_entrance).collect();
                    state.installed_error = None;
                }
                Err(error) => state.installed_error = Some(error),
            }
            Task::none()
        }
        Message::FetchManifest => {
            state.manifest_loading = true;
            state.manifest_error = None;
            Task::perform(fetch_manifest(ctx.core.clone()), Message::ManifestLoaded)
        }
        Message::ManifestLoaded(result) => {
            state.manifest_loading = false;
            match result {
                Ok(data) => {
                    // 默认选中最新正式版，省一步点击。
                    if state.selected_version.is_none() && !data.latest_release.is_empty() {
                        state.selected_version = Some(data.latest_release.clone());
                    }
                    state.latest_release = Some(data.latest_release);
                    state.latest_snapshot = Some(data.latest_snapshot);
                    state.manifest = data.versions;
                    state.manifest_loaded = true;
                    state.manifest_error = None;
                }
                Err(error) => state.manifest_error = Some(error),
            }
            Task::none()
        }
        Message::FilterChanged(filter) => {
            state.filter = filter;
            Task::none()
        }
        Message::SearchChanged(query) => {
            state.search = query;
            Task::none()
        }
        Message::SelectVersion(id) => {
            state.selected_version = Some(id);
            Task::none()
        }
        Message::LoaderChanged(pick) => {
            state.selected_loader = pick;
            Task::none()
        }
        Message::Install => {
            let Some(id) = state.selected_version.clone() else {
                return Task::none();
            };
            if state.installing {
                return Task::none();
            }
            let loader = state.selected_loader;
            state.install_label = format!("正在安装 {}{} …", id, loader.suffix());
            state.installing = true;
            state.install_error = None;
            state.install_done = None;
            state.sweep = new_sweep();
            // 点按弹性：从 PRESS_SCALE 弹回 1.0。
            state.install_pop = Animated::new(PRESS_SCALE, anim::aurora_press());
            state.install_pop.set(1.0);
            Task::perform(
                run_install(ctx.core.clone(), id, loader.choice()),
                Message::Installed,
            )
        }
        Message::Installed(result) => {
            state.installing = false;
            match result {
                Ok(summary) => {
                    state.install_done = Some(summary);
                    // 装完刷新已安装列表，让新版本立即出现在上方。
                    state.installed_loading = true;
                    Task::perform(scan_installed(ctx.core.clone()), Message::InstalledLoaded)
                }
                Err(error) => {
                    state.install_error = Some(error);
                    Task::none()
                }
            }
        }
    }
}

/// 构造第 `i` 枚已安装卡片的入场弹簧（按索引错峰）。
fn build_entrance(i: usize) -> CardEntrance {
    let mut offset = Animated::new(ENTER_OFFSET, anim::aurora_enter());
    let delay = i.min(MAX_STAGGER) as f32 * STAGGER_STEP;
    let released = delay <= 0.0;
    if released {
        offset.set(0.0);
    }
    CardEntrance {
        offset,
        delay,
        released,
    }
}

/// 新建一条扫动进度弹簧（临界阻尼从 0 匀滑到 1，settle 后由 tick 复位形成循环）。
fn new_sweep() -> Animated {
    let mut sweep = Animated::new(0.0, anim::params_from(0.0, SWEEP_DURATION));
    sweep.set(1.0);
    sweep
}

/// 异步扫描本地已安装版本，映射为展示摘要。
async fn scan_installed(core: Arc<Aurora>) -> Result<InstalledScan, String> {
    let scan = core.list_installed().await.map_err(|e| e.to_string())?;
    let versions = scan
        .versions
        .iter()
        .map(|v| InstalledVersion {
            id: v.id.clone(),
            loader: v.loaders.first().map(|l| match &l.version {
                Some(ver) => format!("{} {}", l.kind.as_str(), ver),
                None => l.kind.as_str().to_owned(),
            }),
            is_release: v.is_release(),
        })
        .collect();
    Ok(InstalledScan {
        versions,
        broken: scan.broken.len(),
    })
}

/// 异步抓取远端版本清单，映射为展示摘要。
async fn fetch_manifest(core: Arc<Aurora>) -> Result<ManifestData, String> {
    let manifest = core.list_manifest().await.map_err(|e| e.to_string())?;
    let versions = manifest
        .versions
        .iter()
        .map(|v| ManifestEntry {
            id: v.id.clone(),
            is_release: v.release_type == "release",
            is_snapshot: v.release_type == "snapshot",
        })
        .collect();
    Ok(ManifestData {
        versions,
        latest_release: manifest.latest.release,
        latest_snapshot: manifest.latest.snapshot,
    })
}

/// 异步安装指定版本（可选加载器），返回一句人类可读的完成摘要。
async fn run_install(
    core: Arc<Aurora>,
    id: String,
    loader: Option<LoaderChoice>,
) -> Result<String, String> {
    let outcome = core
        .install(&id, loader, None, None)
        .await
        .map_err(|e| e.to_string())?;
    let mut summary = format!(
        "原版 {} 安装完成：库 {} · 资源 {} · natives {}",
        outcome.vanilla.id, outcome.vanilla.libraries, outcome.vanilla.assets, outcome.vanilla.natives
    );
    if let Some(loader) = outcome.loader {
        summary.push_str(&format!(
            "；{} 加载器就绪（loader {}）",
            loader.id, loader.loader_version
        ));
    }
    Ok(summary)
}

/// 某条目是否匹配当前筛选。
fn filter_matches(filter: VersionFilter, entry: &ManifestEntry) -> bool {
    match filter {
        VersionFilter::Release => entry.is_release,
        VersionFilter::Snapshot => entry.is_snapshot,
        VersionFilter::All => true,
    }
}

/// 渲染页面。
pub fn view<'a>(state: &'a State, ctx: &Ctx) -> Element<'a, Message> {
    let tokens = ctx.tokens();

    let header = page_header(
        "版本",
        "管理已安装版本，安装原版与 Fabric / Quilt 加载器",
        tokens,
    );

    let installed_section = installed_section(state, tokens);
    let install_card = install_card(state, tokens);

    scrollable(
        column![header, installed_section, install_card]
            .spacing(theme::SPACE_LG)
            .padding(theme::SPACE_XL),
    )
    .height(Length::Fill)
    .into()
}

/// 已安装版本区：标题 + 重新扫描 + 卡片列表 / 空态 / 加载 / 错误。
fn installed_section<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let refresh = secondary_button(
        "重新扫描",
        tokens,
        (!state.installed_loading).then_some(Message::RefreshInstalled),
    );
    let head = row![
        container(section_title("已安装版本", tokens)).width(Length::Fill),
        refresh,
    ]
    .align_y(Alignment::Center)
    .spacing(theme::SPACE_MD);

    let body: Element<'a, Message> = if state.installed_loading && state.installed.is_empty() {
        loading("正在扫描已安装版本…", tokens)
    } else if let Some(error) = &state.installed_error {
        text(format!("扫描失败：{error}"))
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted)
            .into()
    } else if state.installed.is_empty() {
        empty_state(
            Icon::Versions,
            "还没有已安装的版本",
            "在下方选择一个版本并安装",
            tokens,
        )
    } else {
        let mut list = column![].spacing(theme::SPACE_SM);
        for (i, v) in state.installed.iter().enumerate() {
            let offset_y = state.cards.get(i).map_or(0.0, |c| c.offset.value());
            list = list.push(installed_view(v, offset_y, tokens));
        }
        if state.broken_count > 0 {
            list = list.push(
                text(format!("另有 {} 个版本目录损坏，未能解析", state.broken_count))
                    .size(theme::TEXT_CAPTION)
                    .color(tokens.on_surface_muted),
            );
        }
        list.into()
    };

    column![head, body].spacing(theme::SPACE_MD).into()
}

/// 单枚已安装版本卡片（滑动入场）：图标 + 版本号/类型 + 加载器标签。
fn installed_view<'a>(v: &'a InstalledVersion, offset_y: f32, tokens: Tokens) -> Element<'a, Message> {
    let marker = if v.is_release { "正式版" } else { "快照" };
    let loader_tag = match v.loader.as_deref() {
        Some(label) => tag(label, tokens, true),
        None => tag("原版", tokens, false),
    };
    let info = column![
        text(v.id.as_str())
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface),
        text(marker)
            .size(theme::TEXT_CAPTION)
            .color(tokens.on_surface_muted),
    ]
    .spacing(2.0)
    .width(Length::Fill);

    let content = row![icon(Icon::Versions, 22.0, tokens.accent_from), info, loader_tag]
        .spacing(theme::SPACE_MD)
        .align_y(Alignment::Center);

    sliding_card(content, (0.0, offset_y), tokens)
}

/// 安装新版本区（毛玻璃卡）：抓清单 + 筛选 + 搜索 + 版本列表 + 加载器 + 安装 + 状态。
fn install_card<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let fetch_label = if state.manifest_loaded {
        "重新获取清单"
    } else {
        "获取版本清单"
    };
    let fetch = secondary_button(
        fetch_label,
        tokens,
        (!state.manifest_loading).then_some(Message::FetchManifest),
    );
    let head = row![
        container(section_title("安装新版本", tokens)).width(Length::Fill),
        fetch,
    ]
    .align_y(Alignment::Center)
    .spacing(theme::SPACE_MD);

    let latest_caption: Element<'a, Message> =
        match (&state.latest_release, &state.latest_snapshot) {
            (Some(release), Some(snapshot)) => {
                text(format!("最新正式版 {release} · 最新快照 {snapshot}"))
                    .size(theme::TEXT_CAPTION)
                    .color(tokens.on_surface_muted)
                    .into()
            }
            _ => column![].into(),
        };

    let filters = row![
        chip(
            "正式版",
            state.filter == VersionFilter::Release,
            tokens,
            Message::FilterChanged(VersionFilter::Release),
        ),
        chip(
            "快照",
            state.filter == VersionFilter::Snapshot,
            tokens,
            Message::FilterChanged(VersionFilter::Snapshot),
        ),
        chip(
            "全部",
            state.filter == VersionFilter::All,
            tokens,
            Message::FilterChanged(VersionFilter::All),
        ),
    ]
    .spacing(theme::SPACE_SM);

    let search = text_input("搜索版本号…", &state.search)
        .on_input(Message::SearchChanged)
        .padding(theme::SPACE_SM)
        .size(theme::TEXT_BODY);

    let list_body = manifest_list(state, tokens);

    let loaders = row![
        chip(
            "原版",
            state.selected_loader == LoaderPick::Vanilla,
            tokens,
            Message::LoaderChanged(LoaderPick::Vanilla),
        ),
        chip(
            "Fabric",
            state.selected_loader == LoaderPick::Fabric,
            tokens,
            Message::LoaderChanged(LoaderPick::Fabric),
        ),
        chip(
            "Quilt",
            state.selected_loader == LoaderPick::Quilt,
            tokens,
            Message::LoaderChanged(LoaderPick::Quilt),
        ),
    ]
    .spacing(theme::SPACE_SM);

    let loader_note = text("Forge、NeoForge 需本地安装器执行 processors，后端暂未接入")
        .size(theme::TEXT_CAPTION)
        .color(tokens.on_surface_muted);

    let can_install = state.selected_version.is_some() && !state.installing;
    let install_button = primary_button(
        "安装所选版本",
        tokens,
        can_install.then_some(Message::Install),
    )
    .width(Length::Fixed(200.0));
    let install_button = spring_press(install_button, state.install_pop.value());

    let target_caption = match &state.selected_version {
        Some(id) => text(format!("目标：{}{}", id, state.selected_loader.suffix())),
        None => text("先在上方选择一个版本".to_owned()),
    }
    .size(theme::TEXT_BODY)
    .color(tokens.on_surface_muted);

    let install_row = row![install_button, target_caption]
        .spacing(theme::SPACE_MD)
        .align_y(Alignment::Center);

    let status = install_status(state, tokens);

    glass_card(
        column![
            head,
            latest_caption,
            filters,
            search,
            list_body,
            text("加载器")
                .size(theme::TEXT_CAPTION)
                .color(tokens.on_surface_muted),
            loaders,
            loader_note,
            install_row,
            status,
        ]
        .spacing(theme::SPACE_MD),
        tokens,
    )
    .into()
}

/// 版本清单列表：按筛选 + 搜索过滤后，取前 `MAX_ROWS` 行放进定高滚动区。
fn manifest_list<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    if state.manifest_loading {
        return loading("正在抓取版本清单…", tokens);
    }
    if let Some(error) = &state.manifest_error {
        return text(format!("抓取失败：{error}"))
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted)
            .into();
    }
    if !state.manifest_loaded {
        return text("点击「获取版本清单」从远端加载可安装版本")
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted)
            .into();
    }

    let query = state.search.trim().to_lowercase();
    let matched: Vec<&ManifestEntry> = state
        .manifest
        .iter()
        .filter(|e| filter_matches(state.filter, e))
        .filter(|e| query.is_empty() || e.id.to_lowercase().contains(&query))
        .collect();

    if matched.is_empty() {
        return text("没有匹配的版本")
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted)
            .into();
    }

    let mut col = column![].spacing(theme::SPACE_XS);
    for entry in matched.iter().take(MAX_ROWS) {
        let selected = state.selected_version.as_deref() == Some(entry.id.as_str());
        col = col.push(version_row(entry, selected, tokens));
    }
    let scroll = scrollable(col).height(Length::Fixed(220.0));

    if matched.len() > MAX_ROWS {
        column![
            scroll,
            text(format!(
                "匹配 {} 项，仅显示前 {}，输入关键词可缩小范围",
                matched.len(),
                MAX_ROWS
            ))
            .size(theme::TEXT_CAPTION)
            .color(tokens.on_surface_muted),
        ]
        .spacing(theme::SPACE_XS)
        .into()
    } else {
        scroll.into()
    }
}

/// 安装状态区：进行中显示扫动进度；完成/失败显示摘要；空闲无占位。
fn install_status<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    if state.installing {
        column![
            loading(state.install_label.as_str(), tokens),
            progress(state.sweep.value(), tokens),
        ]
        .spacing(theme::SPACE_SM)
        .into()
    } else if let Some(done) = &state.install_done {
        text(done.as_str())
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface)
            .into()
    } else if let Some(error) = &state.install_error {
        text(format!("安装失败：{error}"))
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted)
            .into()
    } else {
        column![].into()
    }
}

/// 分段选择用的胶囊按钮：选中态填强调渐变，否则描边幽灵态。
fn chip<'a>(label: &'a str, selected: bool, tokens: Tokens, on_press: Message) -> Element<'a, Message> {
    button(text(label).size(theme::TEXT_BODY))
        .padding([theme::SPACE_XS, theme::SPACE_MD])
        .on_press(on_press)
        .style(move |_theme, status| {
            let (background, text_color) = if selected {
                (
                    Some(Background::Gradient(tokens.accent_linear().into())),
                    tokens.accent_text,
                )
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                (Some(Background::Color(tokens.hover)), tokens.on_surface)
            } else {
                (None, tokens.on_surface_muted)
            };
            button::Style {
                background,
                text_color,
                border: Border {
                    color: if selected {
                        tokens.accent_from
                    } else {
                        tokens.surface_border
                    },
                    width: 1.0,
                    radius: theme::RADIUS_SM.into(),
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// 版本清单里的一行可选版本：左版本号、右类型；选中态填选中底。
fn version_row<'a>(entry: &'a ManifestEntry, selected: bool, tokens: Tokens) -> Element<'a, Message> {
    let kind = if entry.is_release {
        "正式版"
    } else if entry.is_snapshot {
        "快照"
    } else {
        "旧版本"
    };
    let content = row![
        text(entry.id.as_str())
            .size(theme::TEXT_BODY)
            .width(Length::Fill),
        text(kind)
            .size(theme::TEXT_CAPTION)
            .color(tokens.on_surface_muted),
    ]
    .spacing(theme::SPACE_SM)
    .align_y(Alignment::Center);

    button(content)
        .width(Length::Fill)
        .padding([theme::SPACE_SM, theme::SPACE_MD])
        .on_press(Message::SelectVersion(entry.id.clone()))
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
                    radius: theme::RADIUS_SM.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// 小胶囊标签（加载器标记/类型标记）：`accent` 为真用选中底，否则用弱高光底。
fn tag<'a>(label: &'a str, tokens: Tokens, accent: bool) -> Element<'a, Message> {
    let (background, foreground) = if accent {
        (tokens.selected, tokens.on_surface)
    } else {
        (tokens.hover, tokens.on_surface_muted)
    };
    container(
        text(label)
            .size(theme::TEXT_CAPTION)
            .color(foreground),
    )
    .padding([theme::SPACE_XS, theme::SPACE_SM])
    .style(move |_theme| container::Style {
        background: Some(Background::Color(background)),
        border: Border {
            radius: theme::RADIUS_SM.into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// 每帧推进：卡片错峰入场、按钮弹性、安装扫动循环。
pub fn tick(state: &mut State, dt: f32, _ctx: &Ctx) {
    for card in &mut state.cards {
        if card.released {
            card.offset.step(dt);
        } else {
            card.delay -= dt;
            if card.delay <= 0.0 {
                card.offset.set(0.0);
                card.released = true;
            }
        }
    }
    state.install_pop.step(dt);
    if state.installing {
        state.sweep.step(dt);
        if state.sweep.settled() {
            state.sweep = new_sweep();
        }
    }
}

/// 是否仍有动画未收敛：安装进行中，或按钮弹性 / 任一卡片入场未静止。
pub fn animating(state: &State) -> bool {
    state.installing
        || !state.install_pop.settled()
        || state
            .cards
            .iter()
            .any(|c| !c.released || !c.offset.settled())
}
