//! Relay + tunnel: outbound connection loops to the archive backend.
//!
//! `service`/`socket` handle the long-lived WebSocket pairing protocol with
//! foundation.agorix.io. `tunnel` (nested here to keep the fanout rule) runs a
//! `cloudflared` subprocess that exposes the bridge's local HTTP server back
//! out to the public internet — also coordinated via the same archive backend.

pub mod handler;
pub mod service;
pub mod socket;
pub mod tunnel;
pub mod types;

pub use types::*;
