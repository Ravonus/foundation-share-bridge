//! Bulk-archive an artist's Foundation catalog into the local bridge.
//!
//! The bridge calls the archive site's `/api/profile/{username}/browse`
//! endpoint in `mode=foundation`, walks every page, and funnels each result
//! through the existing `pin_work_payload` flow so the watcher picks them up.
//! Progress is mirrored into [`crate::OperationStatus`] so the root page's
//! live-status panel shows real counts.

pub mod handler;
pub mod service;
