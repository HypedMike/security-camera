//! C interface used by the Go program (see `lib/gif/gif_rust.go`).

use crate::{Encoder, Image, shrink_dims, shrink_into};
use std::slice;

/// Shrinks an RGBA frame into `out`, which holds the size `shrink_dims` gives.
///
/// # Safety
/// `data` has `(height - 1) * stride + width * 4` readable bytes and `out`
/// the bytes of the shrunk image.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gif_shrink_rgba(
    data: *const u8,
    len: usize,
    stride: usize,
    width: usize,
    height: usize,
    target: usize,
    out: *mut u8,
) {
    let (_, ow, oh) = shrink_dims(width, height, target);
    let img = Image::Rgba { data: unsafe { slice::from_raw_parts(data, len) }, width, height, stride };
    shrink_into(&img, target, unsafe { slice::from_raw_parts_mut(out, ow * oh * 4) });
}

/// Shrinks a YCbCr frame into `out`.
///
/// # Safety
/// The planes have the lengths their strides and the size imply and `out`
/// the bytes of the shrunk image.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gif_shrink_ycbcr(
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
    target: usize,
    out: *mut u8,
) {
    let (_, ow, oh) = shrink_dims(width, height, target);
    let img = Image::YCbCr {
        y: unsafe { slice::from_raw_parts(y, ylen) },
        cb: unsafe { slice::from_raw_parts(cb, clen) },
        cr: unsafe { slice::from_raw_parts(cr, clen) },
        ystride,
        cstride,
        xs,
        ys,
        width,
        height,
    };
    shrink_into(&img, target, unsafe { slice::from_raw_parts_mut(out, ow * oh * 4) });
}

#[unsafe(no_mangle)]
pub extern "C" fn gif_enc_new(width: usize, height: usize, delay: u16, dither: bool) -> *mut Encoder {
    Box::into_raw(Box::new(Encoder::new(width, height, delay, dither)))
}

/// Adds an RGBA frame of the encoder's size.
///
/// # Safety
/// `enc` comes from `gif_enc_new` and `data` has `(height - 1) * stride + width * 4` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gif_enc_add_rgba(enc: *mut Encoder, data: *const u8, len: usize, stride: usize) {
    let enc = unsafe { &mut *enc };
    let (width, height) = (enc.width, enc.height);
    enc.add(&Image::Rgba { data: unsafe { slice::from_raw_parts(data, len) }, width, height, stride });
}

/// Ends the GIF and frees the encoder. The returned bytes are freed with `gif_free`.
///
/// # Safety
/// `enc` comes from `gif_enc_new` and is not used afterwards; `len` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gif_enc_finish(enc: *mut Encoder, len: *mut usize) -> *mut u8 {
    let bytes = unsafe { Box::from_raw(enc) }.finish().into_boxed_slice();
    unsafe { *len = bytes.len() };
    Box::into_raw(bytes) as *mut u8
}

/// # Safety
/// `ptr` and `len` come from `gif_enc_finish`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gif_free(ptr: *mut u8, len: usize) {
    drop(unsafe { Box::from_raw(slice::from_raw_parts_mut(ptr, len)) });
}
