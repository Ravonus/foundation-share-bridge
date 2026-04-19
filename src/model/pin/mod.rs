//! Pin domain: DTOs, inventory helpers, metadata + dependency extractors,
//! service layer, and the Kubo client wrapper.
//!
//! The re-exports below keep `crate::model::pin::<Name>` working for both the
//! original DTOs and the Stage 5 helpers, so callers don't need to know which
//! sub-module a given item lives in.

pub mod client;
pub mod inventory;
pub mod metadata;
pub mod service;
pub mod types;

pub use types::*;
