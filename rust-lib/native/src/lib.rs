//! Re-exports the C interfaces of the crates in this workspace, see
//! `lib/alarmlog/alarmlog.go`, `lib/detection/background.go` and
//! `lib/gif/gif_rust.go`.

pub use alarmlog::ffi::*;
pub use detection::ffi::*;
pub use gif::ffi::*;
