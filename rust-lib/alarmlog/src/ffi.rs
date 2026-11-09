//! C interface used by the Go program (see `lib/alarmlog/alarmlog.go`).
//!
//! Alarms are named by the id `alarmlog_begin` returns. Strings are bytes
//! and a length, not NUL-terminated.

use crate::{Action, Log};
use std::slice;

/// # Safety
/// `ptr` is null or points to `len` readable bytes.
unsafe fn text<'a>(ptr: *const u8, len: usize) -> std::borrow::Cow<'a, str> {
    if ptr.is_null() || len == 0 {
        return "".into();
    }
    String::from_utf8_lossy(unsafe { slice::from_raw_parts(ptr, len) })
}

#[unsafe(no_mangle)]
pub extern "C" fn alarmlog_new() -> *mut Log {
    Box::into_raw(Box::new(Log::new()))
}

/// # Safety
/// `log` comes from `alarmlog_new` and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alarmlog_free(log: *mut Log) {
    if !log.is_null() {
        drop(unsafe { Box::from_raw(log) });
    }
}

/// Returns the id of the new alarm.
///
/// # Safety
/// `log` comes from `alarmlog_new`; the strings have the lengths given.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alarmlog_begin(
    log: *const Log,
    at: i64,
    utc_offset: i32,
    detector: *const u8,
    detector_len: usize,
    score: f64,
    code: *const u8,
    code_len: usize,
) -> u64 {
    let (detector, code) = unsafe { (text(detector, detector_len), text(code, code_len)) };
    unsafe { &*log }.begin(at, utc_offset, &detector, score, &code)
}

/// # Safety
/// `log` comes from `alarmlog_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alarmlog_message_id(log: *const Log, id: u64) -> i64 {
    unsafe { &*log }.message_id(id)
}

/// # Safety
/// `log` comes from `alarmlog_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alarmlog_posted(log: *const Log, id: u64, message_id: i64) {
    unsafe { &*log }.posted(id, message_id);
}

/// Returns true if the GIF is to be sent now. `failed` says the GIF could not
/// be made, and why.
///
/// # Safety
/// `log` comes from `alarmlog_new`; `reason` has `reason_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alarmlog_finish(
    log: *const Log,
    id: u64,
    failed: bool,
    reason: *const u8,
    reason_len: usize,
) -> bool {
    let reason = unsafe { text(reason, reason_len) };
    unsafe { &*log }.finish(id, failed.then_some(&*reason))
}

/// # Safety
/// `log` comes from `alarmlog_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alarmlog_unsent(log: *const Log, id: u64) {
    unsafe { &*log }.unsent(id);
}

pub const ALARMLOG_UNKNOWN: i32 = 0;
pub const ALARMLOG_SEND: i32 = 1;
pub const ALARMLOG_LATER: i32 = 2;
pub const ALARMLOG_NOTHING: i32 = 3;
pub const ALARMLOG_NO_GIF: i32 = 4;

/// Returns what to do, one of the `ALARMLOG_` constants, and for all but
/// `ALARMLOG_UNKNOWN` writes the id of the alarm to `id`.
///
/// # Safety
/// `log` comes from `alarmlog_new` and `id` is a writable `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alarmlog_request(log: *const Log, message_id: i64, id: *mut u64) -> i32 {
    let (alarm, action) = unsafe { &*log }.request(message_id);
    if let Some(alarm) = alarm {
        unsafe { *id = alarm };
    }
    match action {
        Action::Unknown => ALARMLOG_UNKNOWN,
        Action::Send => ALARMLOG_SEND,
        Action::Later => ALARMLOG_LATER,
        Action::Nothing => ALARMLOG_NOTHING,
        Action::NoGif => ALARMLOG_NO_GIF,
    }
}

/// Describes the latest `n` alarms. The returned UTF-8 text is freed with
/// `alarmlog_free_text`.
///
/// # Safety
/// `log` comes from `alarmlog_new` and `len` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alarmlog_report(log: *const Log, n: usize, len: *mut usize) -> *mut u8 {
    let bytes = unsafe { &*log }.report(n).into_bytes().into_boxed_slice();
    unsafe { *len = bytes.len() };
    Box::into_raw(bytes) as *mut u8
}

/// # Safety
/// `ptr` and `len` come from `alarmlog_report`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alarmlog_free_text(ptr: *mut u8, len: usize) {
    drop(unsafe { Box::from_raw(slice::from_raw_parts_mut(ptr, len)) });
}
