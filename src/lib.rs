//! s7s: a k9s-style TUI (plus a `s7s session` CLI) for searching and resuming
//! Claude Code, Codex, and Antigravity sessions across multiple profiles.
//!
//! This library crate holds every application module so they are testable
//! without starting the binary. The `s7s` binary (`main.rs`) is a thin shim that
//! calls [`run`]; the CLI, TUI event loop, and terminal lifecycle live in
//! `runtime`.

pub mod cache;
pub mod config;
pub mod demo;
pub mod filter;
pub mod handoff;
pub mod model;
pub mod models;
pub mod normalize;
pub mod parser;
pub mod probe;
pub mod profile;
pub mod rename;
pub mod resume;
pub mod scan;
pub mod session_cli;
pub mod session_context;
pub mod theme;
pub mod title;
pub mod ui;
pub mod usage;

mod runtime;

pub use runtime::run;
