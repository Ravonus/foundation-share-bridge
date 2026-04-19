//! Domain modules for the bridge — each child groups the DTOs, service-layer
//! logic, and HTTP handlers for one concern (config, pin, relay, session,
//! system).
//!
//! Persistence note: many types under these modules back on-disk files
//! (`bridge-state.json`, the config file) or the relay WebSocket protocol.
//! Their serde attributes (`rename_all`, `default`, field renames) are
//! load-bearing — changing them silently breaks existing user installs or
//! the relay wire protocol. Treat those wire formats as frozen unless you
//! are intentionally writing a migration.

pub mod config;
pub mod pin;
pub mod relay;
pub mod session;
pub mod system;
pub mod tunnel;
