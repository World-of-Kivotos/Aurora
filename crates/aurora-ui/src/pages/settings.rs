//! 设置页：外观、动画强度、下载源、内存、目录与 Java。
//!
//! 后端现实约束（决定了本页哪些能真正生效）：
//! - `core.config()` 返回 `&AuroraConfig`（只读）；所有配置 setter（`set_game_dir` / `set_client_id`）
//!   都是 `&mut self`，经共享 `Arc<Aurora>` 不可达；`save_config(&self)` 只能把内存里那份配置原样写回。
//!   因此下载源 / 内存 / 并发 / 目录 / Java 自动下载等 config 项，本页只能「读取当前值 + 本地预览编辑」，
//!   无法写回 `config.json`——「保存」按钮据此显式禁用并在文案里说明，绝不静默丢弃用户改动。
//! - 主题 `Mode` 由外壳 `App` 私有持有，页面消息只回流到本页 `update`，没有改全局主题的通道；故主题一栏
//!   只反映当前明暗，切换入口在窗口标题栏。
//! - Java 探测（`aurora_java::detect_all`）未经门面透出，aurora-ui 只依赖 aurora-core，无法列出本机 Java。
//!
//! 真正在本页内闭环生效的交互是「动画强度」：`bounce` / `duration` 双滑条经 [`anim::params_from`] 换算成
//! 弹簧参数，实时驱动一枚预览弹簧（含 [`anim::overshoot_percent`] 过冲回显）。卡片入场用一组错峰弹簧
//! （stagger）驱动上浮，`animating` 精确反映本页是否仍有弹簧在动。

use iced::widget::{column, container, float, row, scrollable, text};
use iced::{Alignment, Background, Border, Element, Length, Task, Vector};

use aurora_core::{AuroraConfig, DownloadSourcePolicy};

use crate::anim::{self, Animated};
use crate::ctx::Ctx;
use crate::theme::{self, Mode, Tokens};
use crate::widgets::{
    labeled_slider, page_header, primary_button, secondary_button, section_title, sliding_card,
};

/// 入场错峰的卡片数量（与 view 中 `sliding_card` 数量一致）。
const ENTER_CARDS: usize = 6;
/// 相邻卡片入场触发的时间差（秒）。
const ENTER_STAGGER: f32 = 0.06;
/// 卡片入场上浮距离（像素，从下方 `ENTER_RISE` 处弹到 0）。
const ENTER_RISE: f32 = 18.0;

/// 动画预览轨道尺寸。
const PREVIEW_W: f32 = 280.0;
const PREVIEW_H: f32 = 28.0;
const PREVIEW_KNOB: f32 = 20.0;
const PREVIEW_PAD: f32 = 4.0;

/// Aurora 默认弹簧手感（与 `anim::aurora()` 同源，供动画强度一栏作恢复默认值）。
const DEFAULT_BOUNCE: f32 = 0.35;
const DEFAULT_DURATION: f32 = 0.30;

/// 最大堆滑条范围（MB）。
const MEMORY_MIN: f32 = 1024.0;
const MEMORY_MAX: f32 = 16384.0;

/// 下载源三档（展示顺序）。
const DOWNLOAD_POLICIES: [DownloadSourcePolicy; 3] = [
    DownloadSourcePolicy::Auto,
    DownloadSourcePolicy::OfficialFirst,
    DownloadSourcePolicy::MirrorFirst,
];

/// 页面状态。config 相关字段自 `core.config()` 播种，仅本地预览（后端暂无 `&self` 写入口）。
#[derive(Debug)]
pub struct State {
    /// 是否已首次进入并开始入场（懒触发：本页首帧 tick 时置真）。
    started: bool,
    /// 每张卡片一枚入场弹簧（值 1→0 驱动上浮位移）。
    enter: Vec<Animated>,
    /// 入场累计时长，用于错峰触发各卡片。
    enter_elapsed: f32,
    /// 已触发上浮的卡片数。
    enter_triggered: usize,
    /// 动画强度预览弹簧（0↔1 往返，参数随 bounce/duration 实时重调）。
    preview: Animated,
    /// 预览弹簧当前目标端（0 或 1，用于往返翻转）。
    preview_target: f32,
    /// 弹跳强度（0 临界阻尼零过冲，越大越弹）。
    bounce: f32,
    /// 到位时长（秒）。
    duration: f32,
    /// 文件下载源策略（本地预览）。
    download_source: DownloadSourcePolicy,
    /// 版本列表源策略（本地预览）。
    version_source: DownloadSourcePolicy,
    /// 最大下载并发（本地预览）。
    concurrency: f32,
    /// 最大堆 MB（本地预览）。
    max_memory_mb: f32,
    /// 最小堆 MB（本地预览，`< 1` 表示不设置）。
    min_memory_mb: f32,
    /// 找不到匹配 Java 时是否自动下载（本地预览）。
    auto_java: bool,
}

impl State {
    /// 以一份配置播种状态：config 项取当前值，动画取 Aurora 默认手感，入场弹簧起于下方偏移。
    fn seed(config: &AuroraConfig) -> Self {
        let enter_params = anim::aurora_enter();
        Self {
            started: false,
            enter: (0..ENTER_CARDS)
                .map(|_| Animated::new(1.0, enter_params))
                .collect(),
            enter_elapsed: 0.0,
            enter_triggered: 0,
            preview: Animated::new(0.0, anim::params_from(DEFAULT_BOUNCE, DEFAULT_DURATION)),
            preview_target: 0.0,
            bounce: DEFAULT_BOUNCE,
            duration: DEFAULT_DURATION,
            download_source: config.download_source,
            version_source: config.version_list_source,
            concurrency: config.download_concurrency as f32,
            max_memory_mb: config.memory.max_mb as f32,
            min_memory_mb: config.memory.min_mb.unwrap_or(0) as f32,
            auto_java: config.auto_download_java,
        }
    }

    /// 当前 bounce/duration 换算出的弹簧参数。
    fn preview_params(&self) -> anim::Params {
        anim::params_from(self.bounce, self.duration)
    }

    /// 滑条调参：给预览换手感；若已静止则顺手往返一次，让用户即时感到差异。
    fn retune_preview(&mut self) {
        let params = self.preview_params();
        self.preview.set_params(params);
        if self.preview.settled() {
            self.preview_target = 1.0 - self.preview_target;
            self.preview.set(self.preview_target);
        }
    }

    /// 主动重播：换手感并强制往返一次。
    fn replay_preview(&mut self) {
        let params = self.preview_params();
        self.preview.set_params(params);
        self.preview_target = 1.0 - self.preview_target;
        self.preview.set(self.preview_target);
    }
}

impl Default for State {
    fn default() -> Self {
        // 仅供 `Pages` 派生 Default 时占位编译；运行期一律走 `init` 的 `seed(core.config())`。
        Self::seed(&AuroraConfig::default())
    }
}

/// 页面消息。
#[derive(Debug, Clone)]
pub enum Message {
    /// 弹跳强度滑条变化。
    BounceChanged(f32),
    /// 到位时长滑条变化。
    DurationChanged(f32),
    /// 重播动画预览。
    ReplayPreview,
    /// 恢复默认手感。
    ResetAnim,
    /// 文件下载源切换。
    DownloadSourceChanged(DownloadSourcePolicy),
    /// 版本列表源切换。
    VersionSourceChanged(DownloadSourcePolicy),
    /// 下载并发滑条变化。
    ConcurrencyChanged(f32),
    /// 最大堆滑条变化。
    MaxMemoryChanged(f32),
    /// 最小堆滑条变化。
    MinMemoryChanged(f32),
    /// Java 自动下载开关。
    AutoJavaChanged(bool),
}

/// 构造页面状态：从当前配置播种。无异步副作用（配置为同步只读）。
pub fn init(ctx: &Ctx) -> (State, Task<Message>) {
    (State::seed(ctx.core.config()), Task::none())
}

/// 处理页面消息。config 项仅落地本地预览（后端无 `&self` 写入口）；动画项即时重调预览弹簧。
pub fn update(state: &mut State, message: Message, _ctx: &Ctx) -> Task<Message> {
    match message {
        Message::BounceChanged(value) => {
            state.bounce = value;
            state.retune_preview();
        }
        Message::DurationChanged(value) => {
            state.duration = value;
            state.retune_preview();
        }
        Message::ReplayPreview => state.replay_preview(),
        Message::ResetAnim => {
            state.bounce = DEFAULT_BOUNCE;
            state.duration = DEFAULT_DURATION;
            state.replay_preview();
        }
        Message::DownloadSourceChanged(policy) => state.download_source = policy,
        Message::VersionSourceChanged(policy) => state.version_source = policy,
        Message::ConcurrencyChanged(value) => state.concurrency = value,
        Message::MaxMemoryChanged(value) => state.max_memory_mb = value,
        Message::MinMemoryChanged(value) => state.min_memory_mb = value,
        Message::AutoJavaChanged(enabled) => state.auto_java = enabled,
    }
    Task::none()
}

/// 渲染页面（整体可滚动，卡片错峰上浮入场）。
pub fn view<'a>(state: &'a State, ctx: &Ctx) -> Element<'a, Message> {
    let tokens = ctx.tokens();
    let config = ctx.core.config();
    let lift = |index: usize| {
        state
            .enter
            .get(index)
            .map(|spring| spring.value())
            .unwrap_or(0.0)
            * ENTER_RISE
    };

    // 一、外观：主题只读反映（切换在标题栏）。
    let appearance = column![
        section_title("外观", tokens),
        field(
            "主题",
            row![
                status_chip("亮色", ctx.mode == Mode::Light, tokens),
                status_chip("暗色", ctx.mode == Mode::Dark, tokens),
            ]
            .spacing(theme::SPACE_SM)
            .into(),
            tokens,
        ),
        hint("在窗口标题栏切换明暗主题。", tokens),
    ]
    .spacing(theme::SPACE_MD);

    // 二、动画强度：本页唯一闭环生效的交互，实时预览过冲与回弹。
    let animation = column![
        section_title("动画强度", tokens),
        hint("调节弹簧手感，实时预览过冲与回弹。", tokens),
        labeled_slider(
            "弹跳 bounce",
            0.0..=0.6,
            state.bounce,
            0.01,
            Message::BounceChanged,
            format!("{:.2}", state.bounce),
            tokens,
        ),
        labeled_slider(
            "时长 duration",
            0.15..=0.5,
            state.duration,
            0.01,
            Message::DurationChanged,
            format!("{:.2}s", state.duration),
            tokens,
        ),
        text(format!(
            "过冲 {:.1}%",
            anim::overshoot_percent(state.bounce)
        ))
        .size(theme::TEXT_BODY)
        .color(tokens.accent_from),
        preview_track(state.preview.value(), tokens),
        row![
            secondary_button("重播预览", tokens, Some(Message::ReplayPreview)),
            secondary_button("恢复默认", tokens, Some(Message::ResetAnim)),
        ]
        .spacing(theme::SPACE_SM),
    ]
    .spacing(theme::SPACE_MD);

    // 三、下载与镜像。
    let mut file_sources = row![].spacing(theme::SPACE_SM);
    for policy in DOWNLOAD_POLICIES {
        file_sources = file_sources.push(choice(
            policy.display_name(),
            state.download_source == policy,
            Message::DownloadSourceChanged(policy),
            tokens,
        ));
    }
    let mut list_sources = row![].spacing(theme::SPACE_SM);
    for policy in DOWNLOAD_POLICIES {
        list_sources = list_sources.push(choice(
            policy.display_name(),
            state.version_source == policy,
            Message::VersionSourceChanged(policy),
            tokens,
        ));
    }
    let download = column![
        section_title("下载与镜像", tokens),
        field("文件下载源", file_sources.into(), tokens),
        field("版本列表源", list_sources.into(), tokens),
        labeled_slider(
            "最大下载线程",
            1.0..=256.0,
            state.concurrency,
            1.0,
            Message::ConcurrencyChanged,
            format!("{} 线程", state.concurrency as u32),
            tokens,
        ),
    ]
    .spacing(theme::SPACE_MD);

    // 四、内存分配。
    let min_text = if state.min_memory_mb < 1.0 {
        "不设置".to_string()
    } else {
        format!("{} MB", state.min_memory_mb as u32)
    };
    let memory = column![
        section_title("内存分配", tokens),
        labeled_slider(
            "最大堆 -Xmx",
            MEMORY_MIN..=MEMORY_MAX,
            state.max_memory_mb,
            256.0,
            Message::MaxMemoryChanged,
            format!("{} MB", state.max_memory_mb as u32),
            tokens,
        ),
        labeled_slider(
            "最小堆 -Xms",
            0.0..=8192.0,
            state.min_memory_mb,
            256.0,
            Message::MinMemoryChanged,
            min_text,
            tokens,
        ),
    ]
    .spacing(theme::SPACE_MD);

    // 五、目录（只读展示当前解析路径）。
    let cache_dir = config
        .cache_directory
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "系统默认".to_string());
    let directories = column![
        section_title("目录", tokens),
        path_row("游戏目录", ctx.core.game_dir().display().to_string(), tokens),
        path_row("数据目录", ctx.core.data_dir().display().to_string(), tokens),
        path_row("缓存目录", cache_dir, tokens),
        hint("切换或新增游戏目录需后端配置写入口，此处暂只读展示。", tokens),
    ]
    .spacing(theme::SPACE_MD);

    // 六、Java 运行时。
    let java = column![
        section_title("Java 运行时", tokens),
        field(
            "找不到匹配 Java 时自动下载",
            row![
                choice("开启", state.auto_java, Message::AutoJavaChanged(true), tokens),
                choice("关闭", !state.auto_java, Message::AutoJavaChanged(false), tokens),
            ]
            .spacing(theme::SPACE_SM)
            .into(),
            tokens,
        ),
        hint(
            "检测本机已安装的 Java 需 aurora-core 暴露探测接口，暂未接入。",
            tokens,
        ),
    ]
    .spacing(theme::SPACE_MD);

    // 保存：后端无 &self 配置写入口，显式禁用并说明，绝不静默吞掉改动。
    let save = column![
        primary_button("保存到 config.json", tokens, None::<Message>),
        hint(
            "以上改动为本地预览。aurora-core 暂无 &self 配置写入口（set_* 均为 &mut self，经共享 Arc 不可达），写回 config.json 需后端支持。",
            tokens,
        ),
    ]
    .spacing(theme::SPACE_SM);

    let body = column![
        page_header("设置", "外观、动画、下载源、内存、目录与 Java", tokens),
        sliding_card(appearance, (0.0, lift(0)), tokens),
        sliding_card(animation, (0.0, lift(1)), tokens),
        sliding_card(download, (0.0, lift(2)), tokens),
        sliding_card(memory, (0.0, lift(3)), tokens),
        sliding_card(directories, (0.0, lift(4)), tokens),
        sliding_card(java, (0.0, lift(5)), tokens),
        save,
    ]
    .spacing(theme::SPACE_LG)
    .padding(theme::SPACE_XL);

    scrollable(body).height(Length::Fill).into()
}

/// 每帧推进：懒触发入场、错峰上浮各卡片、步进预览弹簧。
pub fn tick(state: &mut State, dt: f32, _ctx: &Ctx) {
    state.started = true;

    state.enter_elapsed += dt;
    while state.enter_triggered < state.enter.len()
        && state.enter_elapsed >= state.enter_triggered as f32 * ENTER_STAGGER
    {
        state.enter[state.enter_triggered].set(0.0);
        state.enter_triggered += 1;
    }
    for spring in state.enter.iter_mut() {
        spring.step(dt);
    }

    state.preview.step(dt);
}

/// 是否仍有弹簧未收敛（决定外壳是否继续挂帧）。
pub fn animating(state: &State) -> bool {
    !state.started
        || state.enter_triggered < state.enter.len()
        || state.enter.iter().any(|spring| !spring.settled())
        || !state.preview.settled()
}

// ---- 页面私有小组件 ----

/// 竖排字段：上标签 + 下控件。
fn field<'a>(label: &'a str, control: Element<'a, Message>, tokens: Tokens) -> Element<'a, Message> {
    column![
        text(label).size(theme::TEXT_BODY).color(tokens.on_surface),
        control,
    ]
    .spacing(theme::SPACE_SM)
    .into()
}

/// 次级说明文案（说明约束/去向，非重述控件）。
fn hint<'a>(message: &'a str, tokens: Tokens) -> Element<'a, Message> {
    text(message)
        .size(theme::TEXT_CAPTION)
        .color(tokens.on_surface_muted)
        .into()
}

/// 分段选项按钮：选中走蓝→粉实底主按钮，未选走描边次按钮。
fn choice<'a>(
    label: &'a str,
    selected: bool,
    message: Message,
    tokens: Tokens,
) -> Element<'a, Message> {
    if selected {
        primary_button(label, tokens, Some(message)).into()
    } else {
        secondary_button(label, tokens, Some(message)).into()
    }
}

/// 只读状态芯片：选中走强调渐变底、白字；未选走毛玻璃面、次级文字。无交互。
fn status_chip<'a>(label: &'a str, selected: bool, tokens: Tokens) -> Element<'a, Message> {
    let text_color = if selected {
        tokens.accent_text
    } else {
        tokens.on_surface_muted
    };
    container(text(label).size(theme::TEXT_BODY).color(text_color))
        .padding([theme::SPACE_XS, theme::SPACE_MD])
        .style(chip_style(tokens, selected))
        .into()
}

/// 只读路径行：左固定宽标签 + 右自适应路径（超长自动换行）。
fn path_row<'a>(label: &'a str, value: String, tokens: Tokens) -> Element<'a, Message> {
    row![
        text(label)
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface_muted)
            .width(Length::Fixed(88.0)),
        text(value)
            .size(theme::TEXT_BODY)
            .color(tokens.on_surface)
            .width(Length::Fill),
    ]
    .spacing(theme::SPACE_MD)
    .align_y(Alignment::Center)
    .into()
}

/// 动画预览轨道：固定宽轨 + 强调渐变旋钮，旋钮横向位置由 `pos`（0..1）驱动（弹簧插值）。
fn preview_track<'a>(pos: f32, tokens: Tokens) -> Element<'a, Message> {
    let travel = PREVIEW_W - PREVIEW_KNOB - PREVIEW_PAD * 2.0;
    let offset_x = pos.clamp(0.0, 1.0) * travel;
    let knob = container(text(""))
        .width(Length::Fixed(PREVIEW_KNOB))
        .height(Length::Fixed(PREVIEW_KNOB))
        .style(knob_style(tokens));
    let moving = float(knob).translate(move |_content, _viewport| Vector::new(offset_x, 0.0));
    container(moving)
        .width(Length::Fixed(PREVIEW_W))
        .height(Length::Fixed(PREVIEW_H))
        .padding(PREVIEW_PAD)
        .style(track_style(tokens))
        .into()
}

/// 状态芯片样式：选中强调渐变底，未选毛玻璃面 + 描边。
fn chip_style(tokens: Tokens, selected: bool) -> impl Fn(&iced::Theme) -> container::Style {
    move |_theme| {
        if selected {
            container::Style {
                background: Some(Background::Gradient(tokens.accent_linear().into())),
                border: Border {
                    radius: theme::RADIUS_SM.into(),
                    ..Border::default()
                },
                ..Default::default()
            }
        } else {
            container::Style {
                background: Some(Background::Color(tokens.surface)),
                border: Border {
                    color: tokens.surface_border,
                    width: 1.0,
                    radius: theme::RADIUS_SM.into(),
                },
                ..Default::default()
            }
        }
    }
}

/// 预览轨道样式：毛玻璃面 + 描边，全圆角。
fn track_style(tokens: Tokens) -> impl Fn(&iced::Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(tokens.surface)),
        border: Border {
            color: tokens.surface_border,
            width: 1.0,
            radius: (PREVIEW_H / 2.0).into(),
        },
        ..Default::default()
    }
}

/// 预览旋钮样式：蓝→粉强调渐变实底，全圆角。
fn knob_style(tokens: Tokens) -> impl Fn(&iced::Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Gradient(tokens.accent_linear().into())),
        border: Border {
            radius: (PREVIEW_KNOB / 2.0).into(),
            ..Border::default()
        },
        ..Default::default()
    }
}
