//! Inline JS powering the inventory browser UI on the root dashboard.
//!
//! The JS itself lives in [`inventory.js`] for editor syntax highlighting and
//! to keep this wrapper under the repo's 600-line file rule.

pub const INVENTORY_BROWSER_SCRIPT: &str = concat!("\n", include_str!("inventory.js"));
