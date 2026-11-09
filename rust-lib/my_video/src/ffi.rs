//! C interface used by the Go program (see `lib/my_video/my_video.go`).
//!
//! Strings are bytes and a length, not NUL-terminated. A frame returned by
//! `video_read` is valid until the next `video_read` or `video_close`.

use crate::{Capture, Config, Device, Frame, devices, find, open_any};
use std::slice;
use std::sync::Mutex;

/// # Safety
/// `ptr` is null or points to `len` readable bytes.
unsafe fn text<'a>(ptr: *const u8, len: usize) -> std::borrow::Cow<'a, str> {
    if ptr.is_null() || len == 0 {
        return "".into();
    }
    String::from_utf8_lossy(unsafe { slice::from_raw_parts(ptr, len) })
}

/// Copies as much of `msg` as fits into `buf`, returns the length of `msg`.
///
/// # Safety
/// `buf` is null or has room for `cap` bytes.
unsafe fn put(msg: &str, buf: *mut u8, cap: usize) -> usize {
    if !buf.is_null() {
        let n = msg.len().min(cap);
        unsafe { slice::from_raw_parts_mut(buf, n) }.copy_from_slice(&msg.as_bytes()[..n]);
    }
    msg.len()
}

pub struct DeviceList(Vec<Device>);

#[unsafe(no_mangle)]
pub extern "C" fn video_devices() -> *mut DeviceList {
    Box::into_raw(Box::new(DeviceList(devices())))
}

/// # Safety
/// `list` comes from `video_devices` and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_devices_free(list: *mut DeviceList) {
    if !list.is_null() {
        drop(unsafe { Box::from_raw(list) });
    }
}

/// # Safety
/// `list` comes from `video_devices`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_devices_len(list: *const DeviceList) -> usize {
    unsafe { &*list }.0.len()
}

/// Copies the id of camera `i` into `buf`; returns its length.
///
/// # Safety
/// `list` comes from `video_devices`; `buf` has room for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_devices_id(list: *const DeviceList, i: usize, buf: *mut u8, cap: usize) -> usize {
    match unsafe { &*list }.0.get(i) {
        Some(d) => unsafe { put(&d.id, buf, cap) },
        None => 0,
    }
}

/// Copies the label of camera `i` into `buf`; returns its length.
///
/// # Safety
/// As `video_devices_id`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_devices_label(list: *const DeviceList, i: usize, buf: *mut u8, cap: usize) -> usize {
    match unsafe { &*list }.0.get(i) {
        Some(d) => unsafe { put(&d.label, buf, cap) },
        None => 0,
    }
}

/// Returns the position of the camera the saved label or id names, or -1.
///
/// # Safety
/// `list` comes from `video_devices`; the string has the length given.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_devices_find(list: *const DeviceList, saved: *const u8, len: usize) -> isize {
    let list = &unsafe { &*list }.0;
    let saved = unsafe { text(saved, len) };
    find(list, &saved).and_then(|d| list.iter().position(|x| x == d)).map_or(-1, |i| i as isize)
}

pub struct Video {
    capture: Capture,
    frame: Mutex<Frame>,
    error: Mutex<String>,
}

/// Opens the camera `first` (or the others, see `open_any`). Returns null on
/// failure, with the message copied into `err` and its full length in `err_len`.
///
/// # Safety
/// The strings have the lengths given; `err` has room for `err_cap` bytes and
/// `err_len` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_open(
    first: *const u8,
    first_len: usize,
    width: u32,
    height: u32,
    warmup: usize,
    err: *mut u8,
    err_cap: usize,
    err_len: *mut usize,
) -> *mut Video {
    let first = unsafe { text(first, first_len) };
    let cfg = Config { width, height, warmup_frames: warmup };
    match open_any(&cfg, &first) {
        Ok(capture) => Box::into_raw(Box::new(Video {
            capture,
            frame: Mutex::new(Frame::default()),
            error: Mutex::new(String::new()),
        })),
        Err(e) => {
            unsafe { *err_len = put(&e, err, err_cap) };
            std::ptr::null_mut()
        }
    }
}

/// Waits for the next frame and returns its RGBA pixels, or null if the
/// capture failed (see `video_error`). Only one thread reads at a time.
///
/// # Safety
/// `video` comes from `video_open`; `width` and `height` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_read(video: *const Video, width: *mut u32, height: *mut u32) -> *const u8 {
    let video = unsafe { &*video };
    let mut frame = video.frame.lock().unwrap();
    match video.capture.read(&mut frame) {
        Ok(()) => {
            unsafe {
                *width = frame.width;
                *height = frame.height;
            }
            frame.pixels.as_ptr()
        }
        Err(e) => {
            *video.error.lock().unwrap() = e;
            std::ptr::null()
        }
    }
}

/// Copies why the last `video_read` failed into `buf`; returns its length.
///
/// # Safety
/// `video` comes from `video_open`; `buf` has room for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_error(video: *const Video, buf: *mut u8, cap: usize) -> usize {
    unsafe { put(&(*video).error.lock().unwrap(), buf, cap) }
}

/// Makes a `video_read` that waits, and every later one, fail. Safe to call
/// while another thread is in `video_read`; call `video_close` after.
///
/// # Safety
/// `video` comes from `video_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_stop(video: *const Video) {
    unsafe { &*video }.capture.stop();
}

/// Releases the camera.
///
/// # Safety
/// `video` comes from `video_open`, no call is running and it is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn video_close(video: *mut Video) {
    if !video.is_null() {
        drop(unsafe { Box::from_raw(video) });
    }
}
