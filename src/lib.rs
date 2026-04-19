//! Foundation Share Bridge — library entry point.
//!
//! This crate powers a per-user desktop IPFS pinning companion for the
//! Foundation Archive. The binary in `src/main.rs` is a thin shell that
//! initialises logging and delegates to [`run`].
//!
//! ## Refactor status
//!
//! The codebase is mid-migration from a single 9k-line `main.rs` into focused
//! per-domain modules. During the transition, [`inline`] holds everything that
//! hasn't yet been split out. It is removed entirely in the final stage.

#![forbid(unsafe_code)]

pub(crate) mod app;
pub(crate) mod html;
pub(crate) mod inline;
pub(crate) mod util;

pub use inline::run;
