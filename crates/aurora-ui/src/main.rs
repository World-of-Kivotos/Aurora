//! Aurora 前端（aurora-ui）：iced 0.14 原生启动器外壳。
//!
//! 地基分层（详见 docs/frontend-design.md）：
//! - [`anim`]      弹簧动画内核（半隐式欧拉 + Aurora 手感预设 + [`Animated`](anim::Animated) 封装）。
//! - [`theme`]     亮/暗双主题令牌 + 间距/圆角/字号刻度 + 微软雅黑字体。
//! - [`background`] 应用级极光背景层（亚克力失效兜底，预留换图接口位）。
//! - [`widgets`]   共享组件库（毛玻璃卡片、导航项、按钮、滑条、进度、空态、描边图标）。
//! - [`ctx`]       页面上下文（后端门面句柄 + 当前主题）。
//! - [`pages`]     五个一级页面（各自按固定契约实现，互不相扰）。
//! - [`app`]       顶层 iced 应用外壳：无边框圆角透明窗口 + 亚克力 + 自绘标题栏 + 可展开导航 + 路由。
//!
//! 入口只负责启动 [`app::run`]。

mod anim;
mod app;
mod background;
mod ctx;
mod pages;
mod theme;
mod widgets;

fn main() -> iced::Result {
    app::run()
}
