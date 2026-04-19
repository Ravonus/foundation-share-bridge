//! Cloudflare-tunnel integration. Turns `config.tunnel_enabled` into a
//! running `cloudflared` subprocess that exposes this bridge to the public
//! internet at a unique `<subdomain>.agorix.io` hostname.
//!
//! The web backend holds the Cloudflare API token — this module only talks to
//! `/api/relay/bridge/tunnel/{provision,revoke}` with the device secret and
//! treats the returned tunnel token as an opaque credential to pass straight
//! to `cloudflared tunnel run --token <token>`.

pub mod install;
pub mod service;

pub use service::spawn_tunnel_loop;
