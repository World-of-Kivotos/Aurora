//! 页面上下文 [`Ctx`]：外壳传给每个页面的只读环境——后端门面句柄 + 当前主题。
//!
//! 后端门面 [`aurora_core::Aurora`] 的所有粗粒度操作均为 `&self` 异步方法，故用 `Arc` 共享、克隆廉价；
//! 页面在 `Task::perform` 里克隆出 `Arc` 移进异步块调用后端（见页面异步范式）。`Ctx` 本身每次 update/
//! view 由 app 现造（Arc 克隆 + Mode 拷贝，开销极小），页面不缓存 `Ctx`，主题切换即时生效。

use std::sync::Arc;

use aurora_core::Aurora;

use crate::theme::{Mode, Tokens};

/// 传给页面的运行上下文。
#[derive(Clone)]
pub struct Ctx {
    /// 后端门面句柄（组合下层 crate 的统一入口）。页面所有后端调用都经它。
    pub core: Arc<Aurora>,
    /// 当前主题模式。页面 view 用 [`tokens`](Ctx::tokens) 取令牌着色。
    pub mode: Mode,
}

impl Ctx {
    /// 当前主题的解析令牌（着色用）。
    pub fn tokens(&self) -> Tokens {
        crate::theme::tokens(self.mode)
    }
}
