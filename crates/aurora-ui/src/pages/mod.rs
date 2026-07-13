//! 页面模块集合。每个页面导出固定契约，app 只按契约转发消息与帧、拼装路由，绝不触碰页面内部：
//!
//! ```ignore
//! pub struct State;                                            // 实现 Default
//! pub enum Message;                                            // 该页自有消息，derive(Debug, Clone)
//! pub fn init(ctx: &Ctx) -> (State, Task<Message>);            // 进入应用时构造 + 首个副作用
//! pub fn update(state: &mut State, msg: Message, ctx: &Ctx) -> Task<Message>;
//! pub fn view<'a>(state: &'a State, ctx: &Ctx) -> Element<'a, Message>;
//! pub fn tick(state: &mut State, dt: f32, ctx: &Ctx);         // 每帧推进本页动画（无动画则空实现）
//! pub fn animating(state: &State) -> bool;                     // 是否仍有动画未收敛（决定是否挂帧）
//! ```
//!
//! app 把每页的 Message 包进全局 `Message::<Page>(..)`，用 `Task::map` / `Element::map` 装配回全局；
//! 帧订阅仅在「当前页 `animating()` 或外壳有动画」时挂起，收敛即停。异步调后端见 [`versions`] 的范式。

pub mod accounts;
pub mod home;
pub mod mods;
pub mod settings;
pub mod versions;
