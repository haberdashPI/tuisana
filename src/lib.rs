//! Library entry point for the TUI application.
//!
//! The crate is organized into domain, app, UI, input, config, Asana, and
//! error layers so each concern stays separate and testable.

pub mod app;
pub mod asana;
pub mod config;
pub mod domain;
pub mod error;
pub mod input;
pub mod ui;
