//! C interface used by the Go program (see `lib/recorder/recorder.go`).
//!
//! `recorder_frame` computes the events for a frame and leaves them in the
//! recorder, to be read with `recorder_event` until the next call.

use crate::{Action, Recorder};

#[unsafe(no_mangle)]
pub extern "C" fn recorder_new() -> *mut Recorder {
    Box::into_raw(Box::new(Recorder::new()))
}

/// # Safety
/// `rec` comes from `recorder_new` and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recorder_free(rec: *mut Recorder) {
    if !rec.is_null() {
        drop(unsafe { Box::from_raw(rec) });
    }
}

/// Returns the id of the new clip.
///
/// # Safety
/// `rec` comes from `recorder_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recorder_start(rec: *mut Recorder, at: i64, span: i64, interval: i64) -> u64 {
    unsafe { &mut *rec }.start(at, span, interval)
}

/// Returns how many events there are for the frame.
///
/// # Safety
/// `rec` comes from `recorder_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recorder_frame(rec: *mut Recorder, ts: i64) -> usize {
    unsafe { &mut *rec }.frame(ts).len()
}

/// Reads event `i` of the last `recorder_frame` into `clip`; returns 1 to keep
/// the frame for that clip, 0 if the clip is finished, -1 if there is no such
/// event.
///
/// # Safety
/// `rec` comes from `recorder_new`, `clip` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recorder_event(rec: *const Recorder, i: usize, clip: *mut u64) -> i32 {
    match unsafe { &*rec }.events.get(i) {
        Some(e) => {
            unsafe { *clip = e.clip };
            (e.action == Action::Keep) as i32
        }
        None => -1,
    }
}

/// # Safety
/// `rec` comes from `recorder_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recorder_active(rec: *const Recorder) -> usize {
    unsafe { &*rec }.active()
}
