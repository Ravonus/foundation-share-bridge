//! Root page stylesheet embedded inline in every server-rendered response.
//!
//! The CSS itself lives in [`page.css`] so editors give it syntax highlighting
//! and so this `.rs` wrapper stays under the repo's 600-line file rule.

pub const PAGE_STYLE: &str = concat!("\n", include_str!("page.css"));
