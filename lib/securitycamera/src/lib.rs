//! The security camera: watches a webcam, tells Telegram about motion and
//! answers its commands. `main.rs` only starts [`app::run`]; everything else
//! lives in these modules, each with its own tests.

pub mod alarms;
pub mod app;
pub mod clips;
pub mod commands;
pub mod config;
pub mod dailyreport;
pub mod detector;
pub mod edit_config;
pub mod env;
pub mod httpd;
pub mod livegraph;
pub mod miniapp;
pub mod monitor;
pub mod stream;
pub mod sysinfo;
pub mod syswatch;
pub mod telegram;
pub mod update;
pub mod util;
pub mod waveform;
