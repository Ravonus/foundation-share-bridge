//! Pure utility helpers — no `AppState` / `AppError` dependencies.
//!
//! Anything here is a leaf (or near-leaf depending only on other util
//! modules). Submodules are grouped by concern to keep the directory
//! fanout within the repo-wide 6-child limit.

pub mod data;
pub mod file;
pub mod format;
pub mod text;
pub mod url;
