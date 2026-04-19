//! Render functions — pure HTML assembly from typed data.
//!
//! Every function in this module takes a DTO, a string, or a primitive, and
//! returns a `String` of HTML. No `&AppState`, no I/O, no `.await`. Stage 9
//! will decompose the remaining inline HTML composition in the big handlers
//! (`root_page`, `settings_page`, etc.) and the pieces will land here.

pub mod artist;
pub mod page;
pub mod settings;
pub mod status;
