//! Server-rendered HTML assets: large CSS / JS / SVG string constants.
//!
//! Currently just storage of the big string blocks that the render functions
//! in `inline.rs` reference. In Stage 7 the render functions themselves move
//! here and this module gains a `render/` tree.

pub mod assets;
pub mod scripts;
pub mod styles;
