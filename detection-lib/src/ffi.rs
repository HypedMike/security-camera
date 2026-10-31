//! C interface used by the Go program (see `lib/detection/background_rust.go`).

use crate::{Background, Frame};
use std::slice;

pub const BG_GRAY: i32 = 0;
pub const BG_RGBA: i32 = 1;

#[unsafe(no_mangle)]
pub extern "C" fn bg_new() -> *mut Background {
    Box::into_raw(Box::new(Background::new()))
}

/// # Safety
/// `bg` comes from `bg_new` and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bg_free(bg: *mut Background) {
    if !bg.is_null() {
        drop(unsafe { Box::from_raw(bg) });
    }
}

/// # Safety
/// `bg` comes from `bg_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bg_reset(bg: *mut Background) {
    unsafe { &mut *bg }.reset();
}

/// Scores a frame into `out`. Returns 0 on success, 1 for an empty frame, 2
/// for a buffer too short for the stated size and 3 for an unknown `kind`.
///
/// # Safety
/// `bg` comes from `bg_new`, `data` points to `len` readable bytes and `out`
/// to a writable `f64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bg_score(
    bg: *mut Background,
    kind: i32,
    data: *const u8,
    len: usize,
    width: usize,
    height: usize,
    stride: usize,
    out: *mut f64,
) -> i32 {
    let data = if data.is_null() { &[][..] } else { unsafe { slice::from_raw_parts(data, len) } };
    let frame = match kind {
        BG_GRAY => Frame::Gray { data, width, height, stride },
        BG_RGBA => Frame::Rgba { data, width, height, stride },
        _ => return 3,
    };
    match unsafe { &mut *bg }.score(frame) {
        Ok(s) => {
            unsafe { *out = s };
            0
        }
        Err(crate::Error::EmptyFrame) => 1,
        Err(crate::Error::ShortBuffer) => 2,
    }
}
