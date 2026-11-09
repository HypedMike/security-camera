//! C interface used by the Go program (see `lib/streaming/streaming.go`).

use crate::Ring;
use gif::Image;
use std::slice;

#[unsafe(no_mangle)]
pub extern "C" fn stream_ring_new(capacity: usize) -> *mut Ring {
    Box::into_raw(Box::new(Ring::new(capacity)))
}

/// # Safety
/// `ring` comes from `stream_ring_new` and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stream_ring_free(ring: *mut Ring) {
    if !ring.is_null() {
        drop(unsafe { Box::from_raw(ring) });
    }
}

/// # Safety
/// `ring` comes from `stream_ring_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stream_ring_capacity(ring: *const Ring) -> usize {
    unsafe { &*ring }.capacity()
}

/// # Safety
/// `ring` comes from `stream_ring_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stream_ring_len(ring: *const Ring) -> usize {
    unsafe { &*ring }.len()
}

/// # Safety
/// `ring` comes from `stream_ring_new`; no frame returned by
/// `stream_ring_frame` is used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stream_ring_set_capacity(ring: *mut Ring, n: usize) {
    unsafe { &mut *ring }.set_capacity(n);
}

/// # Safety
/// `ring` comes from `stream_ring_new`, `data` has `(height - 1) * stride + width * 4`
/// bytes; no frame returned by `stream_ring_frame` is used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stream_ring_remember_rgba(
    ring: *mut Ring,
    data: *const u8,
    len: usize,
    stride: usize,
    width: usize,
    height: usize,
) {
    let data = unsafe { slice::from_raw_parts(data, len) };
    unsafe { &mut *ring }.remember(&Image::Rgba { data, width, height, stride });
}

/// # Safety
/// `ring` comes from `stream_ring_new`; the planes have the given lengths and
/// strides; no frame returned by `stream_ring_frame` is used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stream_ring_remember_ycbcr(
    ring: *mut Ring,
    y: *const u8,
    ylen: usize,
    cb: *const u8,
    cr: *const u8,
    clen: usize,
    ystride: usize,
    cstride: usize,
    xs: u32,
    ys: u32,
    width: usize,
    height: usize,
) {
    let img = unsafe {
        Image::YCbCr {
            y: slice::from_raw_parts(y, ylen),
            cb: slice::from_raw_parts(cb, clen),
            cr: slice::from_raw_parts(cr, clen),
            ystride,
            cstride,
            xs,
            ys,
            width,
            height,
        }
    };
    unsafe { &mut *ring }.remember(&img);
}

/// Frame `i` (0 is the oldest): its RGBA bytes, valid until the ring is
/// changed. Null if there is no such frame.
///
/// # Safety
/// `ring` comes from `stream_ring_new`; `width` and `height` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stream_ring_frame(ring: *const Ring, i: usize, width: *mut usize, height: *mut usize) -> *const u8 {
    match unsafe { &*ring }.frame(i) {
        Some((data, w, h)) => {
            unsafe {
                *width = w;
                *height = h;
            }
            data.as_ptr()
        }
        None => std::ptr::null(),
    }
}
