//! Serde data-transfer objects for the bridge.
//!
//! Every HTTP request/response body, relay WebSocket envelope, persistent-state
//! shape, and dashboard query-string binding is declared here. The module is
//! deliberately leaf-level: no IPFS logic, no handlers, no `AppState`
//! interaction — just struct/enum definitions with `serde` attributes.
//!
//! Persistence note: many of these types back on-disk files (`bridge-state.json`,
//! the config file). Their serde attributes (`rename_all`, `default`, field
//! renames) are load-bearing — changing them silently breaks existing user
//! installs. Treat the wire format as frozen unless you are intentionally
//! writing a migration.

pub mod config;
pub mod pin;
pub mod relay;
pub mod session;
pub mod system;
