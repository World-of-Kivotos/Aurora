//! 共享组件库：内建动效/取主题令牌着色的可复用控件，供各页面拼装界面。
//!
//! 分组：
//! - [`icon`]：细线描边图标（自绘，无字体依赖）。
//! - [`card`]：毛玻璃卡片、可滑动卡片、区块标题、页面头、空态占位。
//! - [`button`]：主/次按钮、弹簧按压封装。
//! - [`nav`]：左侧导航项。
//! - [`input`]：带标签滑条。
//! - [`feedback`]：进度条、加载提示。
//!
//! 约定：组件只借用 [`Tokens`](crate::theme::Tokens)（Copy）着色、返回 `Element` 或可继续链式配置的
//! 具体控件类型；不持有状态（需要动画状态的形变由页面持 [`Animated`](crate::anim::Animated) 驱动、把
//! 数值传进组件）。

pub mod button;
pub mod card;
pub mod feedback;
pub mod icon;
pub mod input;
pub mod nav;

pub use button::{primary_button, secondary_button, spring_press};
pub use card::{empty_state, glass_card, page_header, section_title, sliding_card};
pub use feedback::{loading, progress};
pub use icon::{Icon, icon};
pub use input::labeled_slider;
pub use nav::nav_item;
