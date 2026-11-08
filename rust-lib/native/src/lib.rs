//! Re-exports the C interfaces of the crates in this workspace, see
//! `lib/detection/background.go` and `lib/gif/gif_rust.go`.

pub use detection::ffi::*;
pub use gif::ffi::*;
