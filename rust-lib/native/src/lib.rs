//! Re-exports the C interfaces of the crates in this workspace, see
//! `lib/alarmlog/alarmlog.go`, `lib/detection/background.go` and
//! `lib/gif/gif_rust.go`, `lib/streaming/streaming.go`,
//! `lib/recorder/recorder.go`, `lib/my_video/my_video.go`.

pub use alarmlog::ffi::*;
pub use detection::ffi::*;
pub use gif::ffi::*;
pub use my_video::ffi::*;
pub use recorder::ffi::*;
pub use streaming::ffi::*;
