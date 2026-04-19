//! Render functions — pure HTML assembly from typed data.
//!
//! Every function in this module takes a DTO, a string, or a primitive, and
//! returns a `String` of HTML. No `&AppState`, no I/O, no `.await`.

pub mod artist;
pub mod page;
pub mod settings;
pub mod status;
