//! 跨平台 UI 引擎，管理界面树、样式、输入、手势与帧内更新。
//! 使用 Taffy 完成布局，将绘制操作录制为供光栅后端消费的层树。
//! 可选的 `js` 功能通过独立的 JavaScript 运行时接入应用脚本。

pub mod canvas;
mod cast;
mod error;
mod geometry;
pub mod input;
pub mod motion;
pub mod reactive;
pub mod scene;
pub mod semantics;
pub mod session;
pub mod style;

#[cfg(feature = "js")]
#[allow(
    unsafe_code,
    reason = "JS extensions install native bindings into QuickJS"
)]
pub mod js;
pub mod text;
pub mod tree;

#[cfg(test)]
mod barrier_test;
#[cfg(test)]
mod layout_test;
#[cfg(test)]
mod refactor_test;
#[cfg(test)]
mod scroll_test;

pub use error::Error;
pub use geometry::{NodeId, Rect};

#[cfg(test)]
mod style_contract_test;
