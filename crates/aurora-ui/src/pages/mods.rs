//! 资源页：Modrinth + CurseForge 聚合搜索与结果展示。
//!
//! 经 `ctx.core.search`（aurora-core 门面唯一暴露的资源平台入口）发起双源聚合搜索，把领域类型
//! [`aurora_core::SearchHit`] 映射成本页可 Clone 的小摘要 [`HitCard`] 后渲染成毛玻璃结果卡。搜索为
//! 「关键词 + 资源类型」两维；类型切换（模组/资源包/光影/数据包）即时重搜。结果卡带来源标记、下载量、
//! 作者与资源类型标签，进入时按下标错峰弹入（`anim` 弹簧驱动，只做位移不重排）。
//!
//! 后端边界说明（见集成 notes）：aurora-core 门面当前仅提供 `search`，未暴露「安装资源到指定版本」与
//! 「本地 mods 目录扫描 / 启禁 / 删除」。这两项需门面透传 aurora-modplatform 的 `local` 能力后方可接入，
//! 本页不伪造任何不存在的后端调用——安装按钮点击时如实提示接口待接入，本地已装列表暂不呈现。

use std::sync::Arc;

use aurora_core::{Aurora, Platform, ResourceType, SearchHit, SearchQuery};
use iced::widget::{center, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Task};

use crate::anim::{self, Animated};
use crate::ctx::Ctx;
use crate::theme::{self, Tokens};
use crate::widgets::{
    Icon, empty_state, icon, loading, page_header, primary_button, progress, secondary_button,
    sliding_card,
};

/// 结果卡入场的初始纵向位移（像素，向下偏移后弹回 0）。
const REVEAL_OFFSET: f32 = 18.0;
/// 相邻结果卡入场的错峰间隔（秒），营造自上而下的瀑布式弹入。
const STAGGER: f32 = 0.045;
/// 单次展示的结果上限（同后端默认分页 limit，避免一次渲染过多卡片）。
const RESULT_LIMIT: u32 = 30;
/// 卡片描述截断字数，保证卡片高度大致齐整。
const DESCRIPTION_MAX_CHARS: usize = 160;

/// 从 [`SearchHit`] 提炼的小摘要（Message 需 Clone，只留可廉价克隆的字段）。
#[derive(Debug, Clone)]
pub struct HitCard {
    platform: Platform,
    title: String,
    author: Option<String>,
    downloads: u64,
    description: String,
    resource_type: ResourceType,
}

impl HitCard {
    fn from_hit(hit: &SearchHit) -> Self {
        Self {
            platform: hit.platform,
            title: hit.title.clone(),
            author: hit.author.clone(),
            downloads: hit.downloads,
            description: truncate(&hit.description, DESCRIPTION_MAX_CHARS),
            resource_type: hit.resource_type,
        }
    }
}

/// 一次聚合搜索的落地结果（Clone 摘要，供 Message 携带）。
#[derive(Debug, Clone)]
pub struct SearchOutcome {
    hits: Vec<HitCard>,
    /// 单平台失败的说明（另一平台结果仍照常返回；无 key 的 CurseForge 不算失败，不进此列）。
    errors: Vec<String>,
}

/// 单张结果卡的入场动画：错峰延迟 + 位移弹簧。
#[derive(Debug)]
struct CardAnim {
    /// 纵向位移弹簧，从 [`REVEAL_OFFSET`] 收敛到 0。
    reveal: Animated,
    /// 起弹前的剩余延迟（秒），按卡片下标错峰。
    delay: f32,
}

/// 一次成功搜索的结果连同其入场动画。二者同生命周期（结果换新则动画一并重建），故合为一体并在
/// [`State`] 中装箱持有——既让状态语义聚合，也压小 `State` 体量（外壳把每页 State 塞进一个大 enum
/// 变体，扁平铺开会撑大整体尺寸）。
#[derive(Debug)]
struct Loaded {
    /// 结果摘要（命中 + 单平台失败说明）。
    outcome: SearchOutcome,
    /// 与 `outcome.hits` 一一对应的入场动画。
    cards: Vec<CardAnim>,
}

/// 页面状态。
#[derive(Debug, Default)]
pub struct State {
    /// 搜索框内容。
    query: String,
    /// 当前资源类型过滤。
    resource_type: ResourceType,
    /// 是否有搜索在途。
    loading: bool,
    /// CurseForge 是否配置了 API key（决定是否如实提示该源被禁用）。
    curseforge_configured: bool,
    /// 请求代次：过滤快速连点/切类型导致的过期响应。
    request: u64,
    /// 最近一次成功搜索的结果与其入场动画（装箱见 [`Loaded`] 说明）。
    loaded: Option<Box<Loaded>>,
    /// 整次搜索失败的说明（正常路径下几乎不出现，仅覆盖门面返回 Err 的边界）。
    error: Option<String>,
    /// 安装等待后端接口时的如实提示（点击安装按钮触发）。
    notice: Option<String>,
}

/// 页面消息。
#[derive(Debug, Clone)]
pub enum Message {
    /// 搜索框输入变化。
    QueryChanged(String),
    /// 切换资源类型过滤（非空关键词时即时重搜）。
    SetType(ResourceType),
    /// 提交搜索（点击按钮或回车）。
    Submit,
    /// 搜索完成，携带发起时的请求代次与结果。
    Loaded(u64, Result<SearchOutcome, String>),
    /// 点击某结果卡的「安装」。
    Install(usize),
}

/// 构造页面状态与首个副作用。进入不自动联网（同版本页策略），由用户主动搜索触发。
pub fn init(_ctx: &Ctx) -> (State, Task<Message>) {
    let state = State {
        curseforge_configured: curseforge_key_present(),
        ..State::default()
    };
    (state, Task::none())
}

/// 处理页面消息。
pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::QueryChanged(value) => {
            state.query = value;
            Task::none()
        }
        Message::SetType(resource_type) => {
            state.resource_type = resource_type;
            state.notice = None;
            if state.query.trim().is_empty() {
                Task::none()
            } else {
                start_search(state, ctx)
            }
        }
        Message::Submit => {
            if state.query.trim().is_empty() {
                Task::none()
            } else {
                start_search(state, ctx)
            }
        }
        Message::Loaded(request, result) => {
            // 过期响应（其后又发起过新搜索）直接丢弃，避免旧结果覆盖新结果。
            if request != state.request {
                return Task::none();
            }
            state.loading = false;
            match result {
                Ok(outcome) => {
                    let cards = build_card_anims(outcome.hits.len());
                    state.loaded = Some(Box::new(Loaded { outcome, cards }));
                    state.error = None;
                }
                Err(message) => {
                    state.error = Some(message);
                    state.loaded = None;
                }
            }
            Task::none()
        }
        Message::Install(_index) => {
            // 门面当前无资源安装接口：如实告知而非伪造后端调用（详见集成 notes）。
            state.notice = Some(
                "安装到当前版本需 aurora-core 暴露资源安装接口（现门面仅提供 search）。已在集成说明中记录，接口就绪即可接入本按钮。"
                    .to_owned(),
            );
            Task::none()
        }
    }
}

/// 发起一次聚合搜索：置加载态、递增请求代次、克隆句柄进异步块。
fn start_search(state: &mut State, ctx: &Ctx) -> Task<Message> {
    state.loading = true;
    state.error = None;
    state.notice = None;
    state.request += 1;
    let request = state.request;

    let query = SearchQuery::new(state.query.trim())
        .with_resource_type(state.resource_type)
        .with_paging(RESULT_LIMIT, 0);
    let core = ctx.core.clone();
    Task::perform(run_search(core, query), move |result| {
        Message::Loaded(request, result)
    })
}

/// 异步聚合搜索并映射为 Clone 摘要。门面 `&self` 方法经 `Arc<Aurora>` 调用；错误在边界转 `String`。
async fn run_search(core: Arc<Aurora>, query: SearchQuery) -> Result<SearchOutcome, String> {
    let result = core.search(&query).await.map_err(|error| error.to_string())?;
    let hits = result.hits.iter().map(HitCard::from_hit).collect();
    let errors = result
        .errors
        .iter()
        .map(|failure| format!("{} 源暂时不可用：{}", failure.platform.display_name(), failure.error))
        .collect();
    Ok(SearchOutcome { hits, errors })
}

/// 为 `count` 张结果卡构造错峰入场动画（下标越大延迟越久）。
fn build_card_anims(count: usize) -> Vec<CardAnim> {
    (0..count)
        .map(|index| {
            let mut reveal = Animated::new(REVEAL_OFFSET, anim::aurora_enter());
            reveal.set(0.0);
            CardAnim {
                reveal,
                delay: index as f32 * STAGGER,
            }
        })
        .collect()
}

/// 渲染页面。
pub fn view<'a>(state: &'a State, ctx: &Ctx) -> Element<'a, Message> {
    let tokens = ctx.tokens();

    let header = page_header(
        "资源",
        "从 Modrinth 与 CurseForge 聚合搜索 Mod、资源包与光影",
        tokens,
    );
    let type_filter = type_filter(state.resource_type, tokens);
    let search_bar = search_bar(state, tokens);
    let source_note = source_note(state.curseforge_configured, tokens);
    let body = body(state, tokens);

    column![header, type_filter, search_bar, source_note, body]
        .spacing(theme::SPACE_LG)
        .padding(theme::SPACE_XL)
        .width(Fill)
        .height(Fill)
        .into()
}

/// 资源类型分段选择：当前项用主按钮（实底强调），其余用次按钮（描边）。
fn type_filter<'a>(selected: ResourceType, tokens: Tokens) -> Element<'a, Message> {
    let options = [
        (ResourceType::Mod, "模组"),
        (ResourceType::ResourcePack, "资源包"),
        (ResourceType::Shader, "光影"),
        (ResourceType::DataPack, "数据包"),
    ];
    let mut segmented = row![].spacing(theme::SPACE_SM);
    for (resource_type, label) in options {
        let button: Element<'a, Message> = if resource_type == selected {
            primary_button(label, tokens, Some(Message::SetType(resource_type))).into()
        } else {
            secondary_button(label, tokens, Some(Message::SetType(resource_type))).into()
        };
        segmented = segmented.push(button);
    }
    segmented.into()
}

/// 搜索栏：输入框（回车提交）+ 主按钮。搜索在途时按钮禁用并显示进行态文案。
fn search_bar<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    let input = text_input("搜索资源关键词，回车开始", &state.query)
        .on_input(Message::QueryChanged)
        .on_submit(Message::Submit)
        .padding([theme::SPACE_SM, theme::SPACE_MD])
        .size(theme::TEXT_BODY)
        .width(Fill)
        .style(move |_theme, status| input_style(status, tokens));

    let action = if state.loading {
        primary_button("搜索中…", tokens, None)
    } else {
        primary_button("搜索", tokens, Some(Message::Submit))
    };

    row![input, action]
        .spacing(theme::SPACE_SM)
        .align_y(Alignment::Center)
        .into()
}

/// 搜索框样式：毛玻璃面 + 聚焦时强调色描边，前景/占位/选区随主题令牌。
fn input_style(status: text_input::Status, tokens: Tokens) -> text_input::Style {
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
        icon: tokens.icon,
        placeholder: tokens.on_surface_muted,
        value: tokens.on_surface,
        selection: tokens.selected,
    }
}

/// 数据源说明：CurseForge 缺 key 时如实提示仅聚合 Modrinth。
fn source_note<'a>(curseforge_configured: bool, tokens: Tokens) -> Element<'a, Message> {
    let message = if curseforge_configured {
        "已接入 Modrinth 与 CurseForge 双源聚合"
    } else {
        "CurseForge 未配置 API Key，当前仅聚合 Modrinth 结果"
    };
    caption(message, tokens)
}

/// 主体区：按加载 / 出错 / 有结果 / 空结果 / 初始态分支渲染。
fn body<'a>(state: &'a State, tokens: Tokens) -> Element<'a, Message> {
    if state.loading {
        center(
            column![
                loading("正在聚合搜索…", tokens),
                progress(0.35, tokens),
            ]
            .spacing(theme::SPACE_SM)
            .width(Length::Fixed(320.0)),
        )
        .into()
    } else if let Some(error) = &state.error {
        empty_state(Icon::Search, "搜索失败", error.as_str(), tokens)
    } else if let Some(loaded) = &state.loaded {
        if loaded.outcome.hits.is_empty() {
            empty_state(
                Icon::Search,
                "没有找到匹配的资源",
                "换个关键词或切换资源类型再试一次",
                tokens,
            )
        } else {
            results_view(state, loaded, tokens)
        }
    } else {
        empty_state(
            Icon::Mods,
            "搜索资源",
            "输入关键词，从 Modrinth 与 CurseForge 聚合 Mod、资源包与光影",
            tokens,
        )
    }
}

/// 结果列表：可滚动的结果卡序列，顶部附平台失败提示 / 安装接口提示 / 结果计数。
fn results_view<'a>(state: &'a State, loaded: &'a Loaded, tokens: Tokens) -> Element<'a, Message> {
    let mut list = column![].spacing(theme::SPACE_MD).width(Fill);

    for failure in &loaded.outcome.errors {
        list = list.push(caption(failure, tokens));
    }
    if let Some(notice) = &state.notice {
        list = list.push(notice_banner(notice, tokens));
    }
    list = list.push(
        text(format!("找到 {} 个结果", loaded.outcome.hits.len()))
            .size(theme::TEXT_HEADING)
            .color(tokens.on_surface),
    );

    for (index, hit) in loaded.outcome.hits.iter().enumerate() {
        let offset_y = loaded
            .cards
            .get(index)
            .map(|card| card.reveal.value())
            .unwrap_or(0.0);
        list = list.push(sliding_card(hit_card(hit, index, tokens), (0.0, offset_y), tokens));
    }

    scrollable(list).width(Fill).height(Fill).into()
}

/// 单张结果卡内容：图标占位块 + 信息列（标题/来源标记/描述/元数据）+ 安装按钮。
fn hit_card<'a>(hit: &'a HitCard, index: usize, tokens: Tokens) -> Element<'a, Message> {
    let tile = container(icon(Icon::Mods, 30.0, tokens.icon))
        .padding(theme::SPACE_SM)
        .style(move |_theme| container::Style {
            background: Some(Background::Color(tokens.hover)),
            border: Border {
                color: tokens.surface_border,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..container::Style::default()
        });

    let title_row = row![
        text(hit.title.as_str())
            .size(theme::TEXT_HEADING)
            .color(tokens.on_surface),
        source_badge(hit.platform, tokens),
    ]
    .spacing(theme::SPACE_SM)
    .align_y(Alignment::Center);

    let mut meta = row![].spacing(theme::SPACE_MD).align_y(Alignment::Center);
    if let Some(author) = &hit.author {
        meta = meta.push(
            text(format!("作者 {author}"))
                .size(theme::TEXT_CAPTION)
                .color(tokens.on_surface_muted),
        );
    }
    meta = meta.push(
        text(format!("{} 次下载", format_downloads(hit.downloads)))
            .size(theme::TEXT_CAPTION)
            .color(tokens.on_surface_muted),
    );
    meta = meta.push(
        text(resource_type_label(hit.resource_type))
            .size(theme::TEXT_CAPTION)
            .color(tokens.on_surface_muted),
    );

    let info = column![
        title_row,
        text(hit.description.as_str())
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted),
        meta,
    ]
    .spacing(theme::SPACE_XS)
    .width(Fill);

    row![
        tile,
        info,
        secondary_button("安装", tokens, Some(Message::Install(index))),
    ]
    .spacing(theme::SPACE_MD)
    .align_y(Alignment::Center)
    .into()
}

/// 来源标记：Modrinth 取强调蓝、CurseForge 取强调粉，弱底 + 同色描边的胶囊。
fn source_badge<'a>(platform: Platform, tokens: Tokens) -> Element<'a, Message> {
    let color = platform_color(platform, tokens);
    container(
        text(platform.display_name())
            .size(theme::TEXT_CAPTION)
            .color(color),
    )
    .padding([2.0, theme::SPACE_SM])
    .style(move |_theme| container::Style {
        background: Some(Background::Color(color.scale_alpha(0.12))),
        border: Border {
            color,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// 安装接口待接入的提示条：强调色弱底 + 描边，正文用主前景保证可读。
fn notice_banner<'a>(message: &'a str, tokens: Tokens) -> Element<'a, Message> {
    container(
        text(message)
            .size(theme::TEXT_CAPTION)
            .color(tokens.on_surface),
    )
    .padding(theme::SPACE_SM)
    .width(Fill)
    .style(move |_theme| container::Style {
        background: Some(Background::Color(tokens.accent_from.scale_alpha(0.10))),
        border: Border {
            color: tokens.accent_from,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// 次级说明文本（副标题/提示级）。
fn caption<'a>(message: &'a str, tokens: Tokens) -> Element<'a, Message> {
    text(message)
        .size(theme::TEXT_CAPTION)
        .color(tokens.on_surface_muted)
        .into()
}

/// 平台对应的强调色（区分来源）。
fn platform_color(platform: Platform, tokens: Tokens) -> Color {
    match platform {
        Platform::Modrinth => tokens.accent_from,
        Platform::CurseForge => tokens.accent_to,
    }
}

/// 资源类型的中文短标签。
fn resource_type_label(resource_type: ResourceType) -> &'static str {
    match resource_type {
        ResourceType::Mod => "模组",
        ResourceType::Modpack => "整合包",
        ResourceType::ResourcePack => "资源包",
        ResourceType::Shader => "光影",
        ResourceType::DataPack => "数据包",
        ResourceType::Plugin => "插件",
    }
}

/// 把下载量压成人类可读短式（1.2M / 345.0K / 999）。
fn format_downloads(downloads: u64) -> String {
    if downloads >= 1_000_000 {
        format!("{:.1}M", downloads as f64 / 1_000_000.0)
    } else if downloads >= 1_000 {
        format!("{:.1}K", downloads as f64 / 1_000.0)
    } else {
        downloads.to_string()
    }
}

/// 按字符数截断并补省略号（按 char 计，避免切断多字节 UTF-8）。
fn truncate(source: &str, max_chars: usize) -> String {
    let trimmed = source.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_owned()
    } else {
        let head: String = trimmed.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

/// CurseForge 源由后端在缺少 API key 时明确禁用（见 aurora-modplatform `CurseForgeClient::from_env`
/// 读取环境变量 `AURORA_CURSEFORGE_API_KEY`）。aurora-core 未透传该常量，故此处按同一份文档化的环境
/// 变量名判定，仅用于向用户如实提示 CurseForge 是否参与聚合，不参与任何后端调用。
fn curseforge_key_present() -> bool {
    std::env::var("AURORA_CURSEFORGE_API_KEY")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

/// 每帧推进结果卡的错峰入场：延迟未到只递减计时、不位移；延迟耗尽后逐帧推进位移弹簧。
pub fn tick(state: &mut State, dt: f32, _ctx: &Ctx) {
    let Some(loaded) = &mut state.loaded else {
        return;
    };
    for card in &mut loaded.cards {
        if card.delay > 0.0 {
            card.delay -= dt;
            if card.delay <= 0.0 {
                card.reveal.step(dt);
            }
        } else {
            card.reveal.step(dt);
        }
    }
}

/// 是否仍有结果卡在等待入场或位移未收敛。
pub fn animating(state: &State) -> bool {
    state.loaded.as_ref().is_some_and(|loaded| {
        loaded
            .cards
            .iter()
            .any(|card| card.delay > 0.0 || !card.reveal.settled())
    })
}
