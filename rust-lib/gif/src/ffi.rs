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

// The store (see `lib/gif/store.go`). Functions that can fail return a status
// and, unless it is `STORE_OK`, the error text in `*msg`/`*msg_len`, freed
// with `gif_free`.

use crate::store::{Error, Store};

pub const STORE_OK: i32 = 0;
pub const STORE_NOT_FOUND: i32 = 1;
pub const STORE_INVALID_CODE: i32 = 2;
pub const STORE_ERROR: i32 = 3;

/// Hands `bytes` to the caller, to be freed with `gif_free`.
///
/// # Safety
/// `out` and `len` are writable.
unsafe fn give(bytes: Vec<u8>, out: *mut *mut u8, len: *mut usize) {
    let bytes = bytes.into_boxed_slice();
    unsafe {
        *len = bytes.len();
        *out = Box::into_raw(bytes) as *mut u8;
    }
}

/// # Safety
/// `ptr` is null or points to `len` readable bytes.
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 { &[] } else { unsafe { slice::from_raw_parts(ptr, len) } }
}

fn status(e: &Error) -> i32 {
    match e {
        Error::NotFound => STORE_NOT_FOUND,
        Error::InvalidCode => STORE_INVALID_CODE,
        Error::Io(_) => STORE_ERROR,
    }
}

/// Opens the store in the directory `root`. Returns null on failure, with the
/// reason in `*msg`.
///
/// # Safety
/// `root` has `root_len` bytes; `msg` and `msg_len` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gif_store_new(
    root: *const u8,
    root_len: usize,
    max: u64,
    msg: *mut *mut u8,
    msg_len: *mut usize,
) -> *mut Store {
    let root = String::from_utf8_lossy(unsafe { bytes(root, root_len) }).into_owned();
    match Store::new(root, max) {
        Ok(store) => Box::into_raw(Box::new(store)),
        Err(e) => {
            unsafe { give(e.to_string().into_bytes(), msg, msg_len) };
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// `store` comes from `gif_store_new` and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gif_store_free(store: *mut Store) {
    if !store.is_null() {
        drop(unsafe { Box::from_raw(store) });
    }
}

/// # Safety
/// `store` comes from `gif_store_new`; the byte strings have the lengths
/// given; `msg` and `msg_len` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gif_store_save(
    store: *const Store,
    code: *const u8,
    code_len: usize,
    data: *const u8,
    data_len: usize,
    msg: *mut *mut u8,
    msg_len: *mut usize,
) -> i32 {
    let code = String::from_utf8_lossy(unsafe { bytes(code, code_len) });
    match unsafe { &*store }.save(&code, unsafe { bytes(data, data_len) }) {
        Ok(()) => STORE_OK,
        Err(e) => {
            unsafe { give(e.to_string().into_bytes(), msg, msg_len) };
            status(&e)
        }
    }
}

/// On `STORE_OK`, `*out` is the GIF; otherwise the error text.
///
/// # Safety
/// `store` comes from `gif_store_new`; `code` has `code_len` bytes; `out` and
/// `out_len` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gif_store_get(
    store: *const Store,
    code: *const u8,
    code_len: usize,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    let code = String::from_utf8_lossy(unsafe { bytes(code, code_len) });
    match unsafe { &*store }.get(&code) {
        Ok(data) => {
            unsafe { give(data, out, out_len) };
            STORE_OK
        }
        Err(e) => {
            unsafe { give(e.to_string().into_bytes(), out, out_len) };
            status(&e)
        }
    }
}
