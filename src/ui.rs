//! UI rendering modules for the terminal application.
//!
//! The modules split into three layers:
//!
//! - [`theme`], [`text`], and [`date`] are the primitives: semantic styles,
//!   glyphs, width-aware string handling, and relative date formatting. Nothing
//!   above them constructs a raw color.
//! - [`layout`] and [`chrome`] own the frame: where each region goes, and the
//!   header bar, hint bar, status bar, and pane frames that surround content.
//! - [`project_list`], [`task_table`], [`gantt`], [`filter_panel`], [`hints`],
//!   [`help_overlay`], and [`calendar`] render content into a region they are
//!   handed. The last two are centered modals drawn over the panes.
//!   [`gantt`] is the exception to "a region": it hands [`task_table`] spans to
//!   append to each row, so a row and its bar are one line.
//!
//! [`runtime`] runs the event loop and is the only module that talks to the
//! terminal backend.

pub mod calendar;
pub mod chrome;
pub mod date;
pub mod filter_panel;
pub mod filter_sets;
pub mod gantt;
pub mod gantt_order;
pub mod help_overlay;
pub mod hints;
pub mod layout;
pub mod project_list;
pub mod runtime;
pub mod task_table;
pub mod text;
pub mod theme;
