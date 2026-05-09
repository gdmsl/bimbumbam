//! `bimbumbam` — a Wayland fullscreen toddler keyboard-basher.
//!
//! The library crate exposes the testable pieces (color math, particle
//! integration, draw batching, key state machine, CLI parsing). The binary
//! crate (`src/main.rs`) wires them into a Wayland event loop. See the
//! [`wayland`] module for the entry point.

#![warn(rust_2018_idioms)]

pub mod audio;
pub mod color;
pub mod config;
pub mod effect;
pub mod gpu;
pub mod keys;
pub mod particle;
pub mod render;
pub mod text;
pub mod wayland;
