//! 外部 Ability 示例。只依赖 `ability-sdk`，不依赖 frame。
//!
//! 覆盖：typed client / 别名调用 / `call!`、它方事件注入、自举 load、
//! 生命周期、切面、locals、可逆副作用、config、catalog / dep。

mod aspect;
mod chain;
mod ctx_probe;
mod echo;
mod effect;
mod feedback_probe;
mod hub;
mod note;
mod probe;
mod stamp;
mod sub;
mod surface;
mod wrap;

pub use aspect::TraceAspect;
pub use chain::chain_provider;
pub use ctx_probe::ctx_provider;
pub use echo::{EchoExt, echo_provider};
pub use effect::effect_provider;
pub use feedback_probe::feedback_provider;
pub use hub::hub_provider;
pub use note::note_provider;
pub use probe::provider;
pub use stamp::stamp_provider;
pub use sub::sub_provider;
pub use surface::surface_provider;
pub use wrap::{WrapExt, wrap_provider};
