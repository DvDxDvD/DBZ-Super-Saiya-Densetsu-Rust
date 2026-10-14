//! The recompiled code of the game: `include!`s `.work/recomp/generated.rs` if it exists (see
//! `build.rs`), else an empty [`TABLE`]. Nothing here is derived from the ROM until the file is
//! generated, and the file is never tracked.

#![allow(warnings, clippy::all, clippy::pedantic)]

include!(concat!(env!("OUT_DIR"), "/gen.rs"));
