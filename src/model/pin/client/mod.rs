//! State-touching pin-domain I/O: Kubo HTTP, filesystem sync, dependency
//! discovery, and remote pinning service integration.
//!
//! These functions all take `&AppState` (or read a field of it). They live
//! under `pin/` rather than a top-level `ipfs/` module to respect the repo's
//! 6-child folder-fanout rule — the pin domain is the primary consumer.

pub mod discovery;
pub mod kubo;
pub mod remote;
pub mod sync;
