//! Pin service layer — lifecycle bookkeeping, repair/sync loops,
//! work-display assembly. Split across files for the 600-line cap;
//! all items re-exported so `crate::model::pin::service::<name>` works
//! unchanged from callers.

pub mod core;
pub mod handler;
pub mod inventory;
pub mod lifecycle;
pub mod work;

pub use core::*;
pub use inventory::*;
pub use lifecycle::*;
pub use work::*;
