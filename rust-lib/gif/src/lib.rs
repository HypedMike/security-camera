//! Shrinking frames and encoding animated GIFs (port of `lib/gif/gif.go`).
//!
//! Frames are quantized to Go's `palette.Plan9` palette, with the same
//! nearest-colour rules and the same Floyd-Steinberg error diffusion as Go's
//! `image/draw`, so the pictures match the Go encoder's.

use std::sync::OnceLock;

pub mod ffi;
mod lzw;

/// Go's `palette.Plan9`, as R, G, B.
#[rustfmt::skip]
pub const PLAN9: [[u8; 3]; 256] = [
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x44],
    [0x00, 0x00, 0x88],
    [0x00, 0x00, 0xcc],
    [0x00, 0x44, 0x00],
    [0x00, 0x44, 0x44],
    [0x00, 0x44, 0x88],
    [0x00, 0x44, 0xcc],
    [0x00, 0x88, 0x00],
    [0x00, 0x88, 0x44],
    [0x00, 0x88, 0x88],
    [0x00, 0x88, 0xcc],
    [0x00, 0xcc, 0x00],
    [0x00, 0xcc, 0x44],
    [0x00, 0xcc, 0x88],
    [0x00, 0xcc, 0xcc],
    [0x00, 0xdd, 0xdd],
    [0x11, 0x11, 0x11],
    [0x00, 0x00, 0x55],
    [0x00, 0x00, 0x99],
    [0x00, 0x00, 0xdd],
    [0x00, 0x55, 0x00],
    [0x00, 0x55, 0x55],
    [0x00, 0x4c, 0x99],
    [0x00, 0x49, 0xdd],
    [0x00, 0x99, 0x00],
    [0x00, 0x99, 0x4c],
    [0x00, 0x99, 0x99],
    [0x00, 0x93, 0xdd],
    [0x00, 0xdd, 0x00],
    [0x00, 0xdd, 0x49],
    [0x00, 0xdd, 0x93],
    [0x00, 0xee, 0x9e],
    [0x00, 0xee, 0xee],
    [0x22, 0x22, 0x22],
    [0x00, 0x00, 0x66],
    [0x00, 0x00, 0xaa],
    [0x00, 0x00, 0xee],
    [0x00, 0x66, 0x00],
    [0x00, 0x66, 0x66],
    [0x00, 0x55, 0xaa],
    [0x00, 0x4f, 0xee],
    [0x00, 0xaa, 0x00],
    [0x00, 0xaa, 0x55],
    [0x00, 0xaa, 0xaa],
    [0x00, 0x9e, 0xee],
    [0x00, 0xee, 0x00],
    [0x00, 0xee, 0x4f],
    [0x00, 0xff, 0x55],
    [0x00, 0xff, 0xaa],
    [0x00, 0xff, 0xff],
    [0x33, 0x33, 0x33],
    [0x00, 0x00, 0x77],
    [0x00, 0x00, 0xbb],
    [0x00, 0x00, 0xff],
    [0x00, 0x77, 0x00],
    [0x00, 0x77, 0x77],
    [0x00, 0x5d, 0xbb],
    [0x00, 0x55, 0xff],
    [0x00, 0xbb, 0x00],
    [0x00, 0xbb, 0x5d],
    [0x00, 0xbb, 0xbb],
    [0x00, 0xaa, 0xff],
    [0x00, 0xff, 0x00],
    [0x44, 0x00, 0x44],
    [0x44, 0x00, 0x88],
    [0x44, 0x00, 0xcc],
    [0x44, 0x44, 0x00],
    [0x44, 0x44, 0x44],
    [0x44, 0x44, 0x88],
    [0x44, 0x44, 0xcc],
    [0x44, 0x88, 0x00],
    [0x44, 0x88, 0x44],
    [0x44, 0x88, 0x88],
    [0x44, 0x88, 0xcc],
    [0x44, 0xcc, 0x00],
    [0x44, 0xcc, 0x44],
    [0x44, 0xcc, 0x88],
    [0x44, 0xcc, 0xcc],
    [0x44, 0x00, 0x00],
    [0x55, 0x00, 0x00],
    [0x55, 0x00, 0x55],
    [0x4c, 0x00, 0x99],
    [0x49, 0x00, 0xdd],
    [0x55, 0x55, 0x00],
    [0x55, 0x55, 0x55],
    [0x4c, 0x4c, 0x99],
    [0x49, 0x49, 0xdd],
    [0x4c, 0x99, 0x00],
    [0x4c, 0x99, 0x4c],
    [0x4c, 0x99, 0x99],
    [0x49, 0x93, 0xdd],
    [0x49, 0xdd, 0x00],
    [0x49, 0xdd, 0x49],
    [0x49, 0xdd, 0x93],
    [0x49, 0xdd, 0xdd],
    [0x4f, 0xee, 0xee],
    [0x66, 0x00, 0x00],
    [0x66, 0x00, 0x66],
    [0x55, 0x00, 0xaa],
    [0x4f, 0x00, 0xee],
    [0x66, 0x66, 0x00],
    [0x66, 0x66, 0x66],
    [0x55, 0x55, 0xaa],
    [0x4f, 0x4f, 0xee],
    [0x55, 0xaa, 0x00],
    [0x55, 0xaa, 0x55],
    [0x55, 0xaa, 0xaa],
    [0x4f, 0x9e, 0xee],
    [0x4f, 0xee, 0x00],
    [0x4f, 0xee, 0x4f],
    [0x4f, 0xee, 0x9e],
    [0x55, 0xff, 0xaa],
    [0x55, 0xff, 0xff],
    [0x77, 0x00, 0x00],
    [0x77, 0x00, 0x77],
    [0x5d, 0x00, 0xbb],
    [0x55, 0x00, 0xff],
    [0x77, 0x77, 0x00],
    [0x77, 0x77, 0x77],
    [0x5d, 0x5d, 0xbb],
    [0x55, 0x55, 0xff],
    [0x5d, 0xbb, 0x00],
    [0x5d, 0xbb, 0x5d],
    [0x5d, 0xbb, 0xbb],
    [0x55, 0xaa, 0xff],
    [0x55, 0xff, 0x00],
    [0x55, 0xff, 0x55],
    [0x88, 0x00, 0x88],
    [0x88, 0x00, 0xcc],
    [0x88, 0x44, 0x00],
    [0x88, 0x44, 0x44],
    [0x88, 0x44, 0x88],
    [0x88, 0x44, 0xcc],
    [0x88, 0x88, 0x00],
    [0x88, 0x88, 0x44],
    [0x88, 0x88, 0x88],
    [0x88, 0x88, 0xcc],
    [0x88, 0xcc, 0x00],
    [0x88, 0xcc, 0x44],
    [0x88, 0xcc, 0x88],
    [0x88, 0xcc, 0xcc],
    [0x88, 0x00, 0x00],
    [0x88, 0x00, 0x44],
    [0x99, 0x00, 0x4c],
    [0x99, 0x00, 0x99],
    [0x93, 0x00, 0xdd],
    [0x99, 0x4c, 0x00],
    [0x99, 0x4c, 0x4c],
    [0x99, 0x4c, 0x99],
    [0x93, 0x49, 0xdd],
    [0x99, 0x99, 0x00],
    [0x99, 0x99, 0x4c],
    [0x99, 0x99, 0x99],
    [0x93, 0x93, 0xdd],
    [0x93, 0xdd, 0x00],
    [0x93, 0xdd, 0x49],
    [0x93, 0xdd, 0x93],
    [0x93, 0xdd, 0xdd],
    [0x99, 0x00, 0x00],
    [0xaa, 0x00, 0x00],
    [0xaa, 0x00, 0x55],
    [0xaa, 0x00, 0xaa],
    [0x9e, 0x00, 0xee],
    [0xaa, 0x55, 0x00],
    [0xaa, 0x55, 0x55],
    [0xaa, 0x55, 0xaa],
    [0x9e, 0x4f, 0xee],
    [0xaa, 0xaa, 0x00],
    [0xaa, 0xaa, 0x55],
    [0xaa, 0xaa, 0xaa],
    [0x9e, 0x9e, 0xee],
    [0x9e, 0xee, 0x00],
    [0x9e, 0xee, 0x4f],
    [0x9e, 0xee, 0x9e],
    [0x9e, 0xee, 0xee],
    [0xaa, 0xff, 0xff],
    [0xbb, 0x00, 0x00],
    [0xbb, 0x00, 0x5d],
    [0xbb, 0x00, 0xbb],
    [0xaa, 0x00, 0xff],
    [0xbb, 0x5d, 0x00],
    [0xbb, 0x5d, 0x5d],
    [0xbb, 0x5d, 0xbb],
    [0xaa, 0x55, 0xff],
    [0xbb, 0xbb, 0x00],
    [0xbb, 0xbb, 0x5d],
    [0xbb, 0xbb, 0xbb],
    [0xaa, 0xaa, 0xff],
    [0xaa, 0xff, 0x00],
    [0xaa, 0xff, 0x55],
    [0xaa, 0xff, 0xaa],
    [0xcc, 0x00, 0xcc],
    [0xcc, 0x44, 0x00],
    [0xcc, 0x44, 0x44],
    [0xcc, 0x44, 0x88],
    [0xcc, 0x44, 0xcc],
    [0xcc, 0x88, 0x00],
    [0xcc, 0x88, 0x44],
    [0xcc, 0x88, 0x88],
    [0xcc, 0x88, 0xcc],
    [0xcc, 0xcc, 0x00],
    [0xcc, 0xcc, 0x44],
    [0xcc, 0xcc, 0x88],
    [0xcc, 0xcc, 0xcc],
    [0xcc, 0x00, 0x00],
    [0xcc, 0x00, 0x44],
    [0xcc, 0x00, 0x88],
    [0xdd, 0x00, 0x93],
    [0xdd, 0x00, 0xdd],
    [0xdd, 0x49, 0x00],
    [0xdd, 0x49, 0x49],
    [0xdd, 0x49, 0x93],
    [0xdd, 0x49, 0xdd],
    [0xdd, 0x93, 0x00],
    [0xdd, 0x93, 0x49],
    [0xdd, 0x93, 0x93],
    [0xdd, 0x93, 0xdd],
    [0xdd, 0xdd, 0x00],
    [0xdd, 0xdd, 0x49],
    [0xdd, 0xdd, 0x93],
    [0xdd, 0xdd, 0xdd],
    [0xdd, 0x00, 0x00],
    [0xdd, 0x00, 0x49],
    [0xee, 0x00, 0x4f],
    [0xee, 0x00, 0x9e],
    [0xee, 0x00, 0xee],
    [0xee, 0x4f, 0x00],
    [0xee, 0x4f, 0x4f],
    [0xee, 0x4f, 0x9e],
    [0xee, 0x4f, 0xee],
    [0xee, 0x9e, 0x00],
    [0xee, 0x9e, 0x4f],
    [0xee, 0x9e, 0x9e],
    [0xee, 0x9e, 0xee],
    [0xee, 0xee, 0x00],
    [0xee, 0xee, 0x4f],
    [0xee, 0xee, 0x9e],
    [0xee, 0xee, 0xee],
    [0xee, 0x00, 0x00],
    [0xff, 0x00, 0x00],
    [0xff, 0x00, 0x55],
    [0xff, 0x00, 0xaa],
    [0xff, 0x00, 0xff],
    [0xff, 0x55, 0x00],
    [0xff, 0x55, 0x55],
    [0xff, 0x55, 0xaa],
    [0xff, 0x55, 0xff],
    [0xff, 0xaa, 0x00],
    [0xff, 0xaa, 0x55],
    [0xff, 0xaa, 0xaa],
    [0xff, 0xaa, 0xff],
    [0xff, 0xff, 0x00],
    [0xff, 0xff, 0x55],
    [0xff, 0xff, 0xaa],
    [0xff, 0xff, 0xff],
];

/// A borrowed source frame.
#[derive(Clone, Copy)]
pub enum Image<'a> {
    /// Packed 8-bit R, G, B, A (alpha ignored: camera frames are opaque).
    Rgba { data: &'a [u8], width: usize, height: usize, stride: usize },
    /// Planar Y, Cb, Cr. `xs` and `ys` are the log2 chroma subsampling
    /// factors (4:2:2 is xs=1, ys=0; 4:2:0 is xs=1, ys=1).
    YCbCr {
        y: &'a [u8],
        cb: &'a [u8],
        cr: &'a [u8],
        ystride: usize,
        cstride: usize,
        xs: u32,
        ys: u32,
        width: usize,
        height: usize,
    },
}

impl Image<'_> {
    pub fn size(&self) -> (usize, usize) {
        match *self {
            Image::Rgba { width, height, .. } | Image::YCbCr { width, height, .. } => (width, height),
        }
    }

    /// 16-bit R, G, B of the pixel, like Go's `Color.RGBA()`.
    #[inline]
    fn rgb16(&self, x: usize, y: usize) -> [u32; 3] {
        match *self {
            Image::Rgba { data, stride, .. } => {
                let p = &data[y * stride + x * 4..];
                [p[0] as u32 * 0x101, p[1] as u32 * 0x101, p[2] as u32 * 0x101]
            }
            Image::YCbCr { y: yp, cb, cr, ystride, cstride, xs, ys, .. } => {
                let ci = (y >> ys) * cstride + (x >> xs);
                ycbcr16(yp[y * ystride + x], cb[ci], cr[ci])
            }
        }
    }
}

/// Go's `color.YCbCr.RGBA`.
#[inline]
fn ycbcr16(y: u8, cb: u8, cr: u8) -> [u32; 3] {
    let yy1 = y as i32 * 0x10101;
    let cb1 = cb as i32 - 128;
    let cr1 = cr as i32 - 128;
    let ch = |v: i32| -> u32 {
        if (v as u32) & 0xff00_0000 == 0 { (v >> 8) as u32 } else { (!(v >> 31) & 0xffff) as u32 }
    };
    [
        ch(yy1 + 91881 * cr1),
        ch(yy1 - 22554 * cb1 - 46802 * cr1),
        ch(yy1 + 116130 * cb1),
    ]
}

/// An owned, tightly packed RGBA image (alpha always 0xff).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    pub pix: Vec<u8>,
}

/// The factor `shrink` scales by and the size of the result.
pub fn shrink_dims(width: usize, height: usize, target: usize) -> (usize, usize, usize) {
    let f = if target > 0 { ((width + target - 1) / target).max(1) } else { 1 };
    (f, width / f, height / f)
}

/// Scales `img` down by the smallest integer factor that makes it at most
/// `target` wide, averaging each block of pixels. A `target` of 0 keeps the
/// original size.
pub fn shrink(img: &Image, target: usize) -> RgbaImage {
    let (w, h) = img.size();
    let (_, ow, oh) = shrink_dims(w, h, target);
    let mut pix = vec![0u8; ow * oh * 4];
    shrink_into(img, target, &mut pix);
    RgbaImage { width: ow, height: oh, pix }
}

/// `shrink` into `out`, which holds `ow * oh * 4` bytes (see `shrink_dims`).
pub fn shrink_into(img: &Image, target: usize, out: &mut [u8]) {
    let (w, h) = img.size();
    let (f, ow, oh) = shrink_dims(w, h, target);
    let n = (f * f) as u32;
    for oy in 0..oh {
        let row = &mut out[oy * ow * 4..(oy + 1) * ow * 4];
        match *img {
            // the common cases, without the per-pixel match
            Image::Rgba { data, stride, .. } => {
                for (ox, px) in row.chunks_exact_mut(4).enumerate() {
                    let mut s = [0u32; 3];
                    for dy in 0..f {
                        let base = (oy * f + dy) * stride + ox * f * 4;
                        for p in data[base..base + f * 4].chunks_exact(4) {
                            s[0] += p[0] as u32;
                            s[1] += p[1] as u32;
                            s[2] += p[2] as u32;
                        }
                    }
                    // Go averages the 16-bit values (8-bit * 0x101) and keeps the top 8 bits
                    px[0] = ((s[0] * 0x101 / n) >> 8) as u8;
                    px[1] = ((s[1] * 0x101 / n) >> 8) as u8;
                    px[2] = ((s[2] * 0x101 / n) >> 8) as u8;
                    px[3] = 0xff;
                }
            }
            Image::YCbCr { .. } => {
                for (ox, px) in row.chunks_exact_mut(4).enumerate() {
                    let mut s = [0u32; 3];
                    for dy in 0..f {
                        for dx in 0..f {
                            let c = img.rgb16(ox * f + dx, oy * f + dy);
                            s[0] += c[0];
                            s[1] += c[1];
                            s[2] += c[2];
                        }
                    }
                    px[0] = ((s[0] / n) >> 8) as u8;
                    px[1] = ((s[1] / n) >> 8) as u8;
                    px[2] = ((s[2] / n) >> 8) as u8;
                    px[3] = 0xff;
                }
            }
        }
    }
}

/// Palette index for each colour reduced to 5 bits per channel: the closest
/// Plan9 entry to the centre of the cell (what Go's `quantize` uses).
fn lut() -> &'static [u8; 1 << 15] {
    static LUT: OnceLock<Box<[u8; 1 << 15]>> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = Box::new([0u8; 1 << 15]);
        for (i, e) in lut.iter_mut().enumerate() {
            let c = [
                ((i >> 10) as u32) << 3 | 4,
                ((i >> 5 & 31) as u32) << 3 | 4,
                ((i & 31) as u32) << 3 | 4,
            ];
            *e = nearest(c[0] * 0x101, c[1] * 0x101, c[2] * 0x101) as u8;
        }
        lut
    })
}

#[inline]
fn sq_diff(x: u32, y: u32) -> u32 {
    let d = x.abs_diff(y);
    (d * d) >> 2
}

/// Closest palette entry in 16-bit RGB, the first one on ties (Go's `Palette.Index`).
#[inline]
fn nearest(r: u32, g: u32, b: u32) -> usize {
    let mut best = (0usize, u32::MAX);
    for (i, p) in PLAN9.iter().enumerate() {
        let sum = sq_diff(r, p[0] as u32 * 0x101) + sq_diff(g, p[1] as u32 * 0x101) + sq_diff(b, p[2] as u32 * 0x101);
        if sum < best.1 {
            best = (i, sum);
            if sum == 0 {
                break;
            }
        }
    }
    best.0
}

/// Quantizes `img` to palette indices with no dithering.
fn quantize(img: &Image, out: &mut [u8]) {
    let lut = lut();
    let (w, h) = img.size();
    match *img {
        Image::Rgba { data, stride, .. } => {
            for y in 0..h {
                let src = &data[y * stride..y * stride + w * 4];
                let dst = &mut out[y * w..(y + 1) * w];
                for (d, p) in dst.iter_mut().zip(src.chunks_exact(4)) {
                    *d = lut[(p[0] as usize >> 3) << 10 | (p[1] as usize >> 3) << 5 | (p[2] as usize >> 3)];
                }
            }
        }
        Image::YCbCr { .. } => {
            for y in 0..h {
                for x in 0..w {
                    let c = img.rgb16(x, y);
                    out[y * w + x] = nearest(c[0], c[1], c[2]) as u8;
                }
            }
        }
    }
}

/// Quantizes `img` to palette indices with Floyd-Steinberg error diffusion,
/// the same arithmetic as Go's `draw.FloydSteinberg`.
fn dither(img: &Image, out: &mut [u8]) {
    let (w, h) = img.size();
    let pal16: Vec<[i32; 3]> =
        PLAN9.iter().map(|p| [p[0] as i32 * 0x101, p[1] as i32 * 0x101, p[2] as i32 * 0x101]).collect();
    let mut curr = vec![[0i32; 3]; w + 2];
    let mut next = vec![[0i32; 3]; w + 2];
    for y in 0..h {
        for x in 0..w {
            let s = img.rgb16(x, y);
            let mut e = [0i32; 3];
            for c in 0..3 {
                e[c] = (s[c] as i32 + curr[x + 1][c] / 16).clamp(0, 0xffff);
            }
            // all 256 distances first (no early exit, so it vectorizes), then the
            // first minimum, which is the entry Go's search settles on
            let mut sums = [0u32; 256];
            for (sum, p) in sums.iter_mut().zip(&pal16) {
                *sum = sq_diff(e[0] as u32, p[0] as u32)
                    + sq_diff(e[1] as u32, p[1] as u32)
                    + sq_diff(e[2] as u32, p[2] as u32);
            }
            let min = sums.iter().copied().min().unwrap();
            let best = (sums.iter().position(|&s| s == min).unwrap(), min);
            out[y * w + x] = best.0 as u8;
            for c in 0..3 {
                let err = e[c] - pal16[best.0][c];
                next[x][c] += err * 3;
                next[x + 1][c] += err * 5;
                next[x + 2][c] += err;
                curr[x + 2][c] += err * 7;
            }
        }
        std::mem::swap(&mut curr, &mut next);
        next.fill([0; 3]);
    }
}

/// Builds an animated GIF frame by frame. All frames have the same size.
pub struct Encoder {
    width: usize,
    height: usize,
    delay: u16,
    dither: bool,
    out: Vec<u8>,
    indices: Vec<u8>,
    frames: usize,
}

impl Encoder {
    /// `delay` is the pause between frames in hundredths of a second.
    pub fn new(width: usize, height: usize, delay: u16, dither: bool) -> Self {
        let mut out = Vec::with_capacity(64 * 1024);
        out.extend_from_slice(b"GIF89a");
        out.extend_from_slice(&(width as u16).to_le_bytes());
        out.extend_from_slice(&(height as u16).to_le_bytes());
        out.extend_from_slice(&[0xf7, 0, 0]); // 256-colour global table
        for p in &PLAN9 {
            out.extend_from_slice(p);
        }
        // loop forever
        out.extend_from_slice(&[0x21, 0xff, 0x0b]);
        out.extend_from_slice(b"NETSCAPE2.0");
        out.extend_from_slice(&[3, 1, 0, 0, 0]);
        Encoder { width, height, delay, dither, out, indices: vec![0; width * height], frames: 0 }
    }

    /// Adds a frame, which must be `width` x `height`.
    pub fn add(&mut self, img: &Image) {
        assert_eq!(img.size(), (self.width, self.height), "gif: frames differ in size");
        if self.dither {
            dither(img, &mut self.indices);
        } else {
            quantize(img, &mut self.indices);
        }
        let o = &mut self.out;
        o.extend_from_slice(&[0x21, 0xf9, 4, 0]);
        o.extend_from_slice(&self.delay.to_le_bytes());
        o.extend_from_slice(&[0, 0]);
        o.push(0x2c);
        o.extend_from_slice(&[0, 0, 0, 0]);
        o.extend_from_slice(&(self.width as u16).to_le_bytes());
        o.extend_from_slice(&(self.height as u16).to_le_bytes());
        o.push(0);
        lzw::encode(&self.indices, o);
        self.frames += 1;
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.out.push(0x3b);
        self.out
    }
}

/// Builds an animated GIF from `images`, which must all be the same size.
/// `None` if there are none.
pub fn encode(images: &[Image], delay: u16, dither: bool) -> Option<Vec<u8>> {
    let (w, h) = images.first()?.size();
    let mut enc = Encoder::new(w, h, delay, dither);
    for img in images {
        enc.add(img);
    }
    Some(enc.finish())
}
