//! Background-model motion detection (port of `lib/detection/background.go`).
//!
//! Each frame is shrunk to a small grayscale image and lightly blurred. Every
//! pixel keeps a running mean and variance of its brightness; a pixel has
//! changed when it is further from its mean than `k` standard deviations (and
//! at least `min_diff`). Isolated changed pixels are dropped as noise and the
//! rest are grouped into connected blobs. The score of a frame is the area of
//! its largest blob as a fraction of the frame.
//!
//! When more than `lighting` of the frame changes at once the frame scores 0
//! and the model is learned again for `settle` frames. The returned score is
//! the `persist`-th largest of the last `window` frame scores.

use std::fmt;

/// A borrowed frame. `stride` is the length in bytes of one row.
#[derive(Clone, Copy)]
pub enum Frame<'a> {
    /// A brightness plane, one byte per pixel (e.g. the Y plane of YCbCr).
    Gray { data: &'a [u8], width: usize, height: usize, stride: usize },
    /// Packed 8-bit R, G, B, A.
    Rgba { data: &'a [u8], width: usize, height: usize, stride: usize },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    EmptyFrame,
    ShortBuffer,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EmptyFrame => f.write_str("detection: frame is empty"),
            Error::ShortBuffer => f.write_str("detection: frame buffer is too short"),
        }
    }
}

impl std::error::Error for Error {}

const SETTLE_ALPHA: f32 = 0.3;
const INIT_VARIANCE: f32 = 25.0; // (5 brightness levels)²
const MIN_VARIANCE: f32 = 4.0;

pub struct Background {
    /// Width frames are shrunk to before detection.
    pub width: usize,
    /// Standard deviations from its mean a pixel must be to count as changed.
    pub k: f64,
    /// Smallest brightness change (0 to 255) that counts as changed.
    pub min_diff: f64,
    /// How fast unchanged pixels learn the scene (0 to 1 per frame).
    pub alpha: f64,
    /// How fast changed pixels learn the scene.
    pub slow_alpha: f64,
    /// Fraction of changed pixels above which the frame is a lighting change.
    pub lighting: f64,
    /// Frames the model is learned for, without scoring, at start and after a lighting change.
    pub settle: usize,
    pub persist: usize,
    pub window: usize,

    w: usize,
    h: usize,
    scale: usize,
    sums: Vec<u32>,
    gray: Vec<f32>,
    tmp: Vec<f32>,
    mean: Vec<f32>,
    vari: Vec<f32>,
    changed: Vec<bool>,
    keep: Vec<bool>,
    stack: Vec<u32>,
    learned: bool,
    settle_left: usize,
    scores: Vec<f64>,
    next: usize,
}

impl Default for Background {
    fn default() -> Self {
        Self::new()
    }
}

impl Background {
    pub fn new() -> Self {
        Background {
            width: 320,
            k: 3.0,
            min_diff: 12.0,
            alpha: 0.02,
            slow_alpha: 0.002,
            lighting: 0.5,
            settle: 10,
            persist: 3,
            window: 5,
            w: 0,
            h: 0,
            scale: 0,
            sums: Vec::new(),
            gray: Vec::new(),
            tmp: Vec::new(),
            mean: Vec::new(),
            vari: Vec::new(),
            changed: Vec::new(),
            keep: Vec::new(),
            stack: Vec::new(),
            learned: false,
            settle_left: 0,
            scores: Vec::new(),
            next: 0,
        }
    }

    /// Forgets the learned scene: the next frame starts learning it again.
    pub fn reset(&mut self) {
        self.w = 0;
        self.h = 0;
    }

    pub fn score(&mut self, frame: Frame) -> Result<f64, Error> {
        self.load(frame)?;
        if self.gray.is_empty() {
            return Err(Error::EmptyFrame);
        }
        if !self.learned {
            self.start();
            return Ok(self.push(0.0));
        }
        let n = self.gray.len();

        if self.settle_left > 0 {
            self.settle_left -= 1;
            self.learn_all(SETTLE_ALPHA);
            return Ok(self.push(0.0));
        }

        let k2 = (self.k * self.k) as f32;
        let min2 = (self.min_diff * self.min_diff) as f32;
        let mut count = 0usize;
        for (((c, &g), &m), &v) in self
            .changed
            .iter_mut()
            .zip(&self.gray)
            .zip(&self.mean)
            .zip(&self.vari)
        {
            let d = g - m;
            let d2 = d * d;
            *c = d2 > k2 * v && d2 > min2;
            count += *c as usize;
        }

        if count as f64 > self.lighting * n as f64 {
            self.settle_left = self.settle;
            self.learn_all(SETTLE_ALPHA);
            return Ok(self.push(0.0));
        }

        let alpha = self.alpha as f32;
        let slow = self.slow_alpha as f32;
        for (((&c, &g), m), v) in self
            .changed
            .iter()
            .zip(&self.gray)
            .zip(self.mean.iter_mut())
            .zip(self.vari.iter_mut())
        {
            if c {
                // only the mean: learning the variance from the object itself
                // would hide it within seconds
                *m += slow * (g - *m);
            } else {
                learn(m, v, g, alpha);
            }
        }

        self.despeckle();
        let blob = self.largest_blob();
        Ok(self.push(blob as f64 / n as f64))
    }

    /// Shrinks `frame` into `self.gray`, reallocating (and so resetting the
    /// model) when the frame size or `width` changed.
    fn load(&mut self, frame: Frame) -> Result<(), Error> {
        let (fw, fh) = match frame {
            Frame::Gray { width, height, .. } | Frame::Rgba { width, height, .. } => (width, height),
        };
        let scale = ((fw + self.width.max(1) - 1) / self.width.max(1)).max(1);
        let (w, h) = (fw / scale, fh / scale);
        if w != self.w || h != self.h || scale != self.scale {
            self.w = w;
            self.h = h;
            self.scale = scale;
            let n = w * h;
            self.sums = vec![0; n];
            self.gray = vec![0.0; n];
            self.tmp = vec![0.0; n];
            self.changed = vec![false; n];
            self.keep = vec![false; n];
            self.learned = false;
        }
        if w == 0 || h == 0 {
            return Ok(());
        }

        let (rows, cols) = (h * scale, w * scale);
        self.sums.fill(0);
        match frame {
            Frame::Gray { data, stride, .. } => {
                if stride < cols || data.len() < (rows - 1) * stride + cols {
                    return Err(Error::ShortBuffer);
                }
                for y in 0..rows {
                    let row = &data[y * stride..y * stride + cols];
                    let out = &mut self.sums[(y / scale) * w..][..w];
                    for (o, cell) in out.iter_mut().zip(row.chunks_exact(scale)) {
                        *o += cell.iter().map(|&p| p as u32).sum::<u32>();
                    }
                }
            }
            Frame::Rgba { data, stride, .. } => {
                if stride < cols * 4 || data.len() < (rows - 1) * stride + cols * 4 {
                    return Err(Error::ShortBuffer);
                }
                for y in 0..rows {
                    let row = &data[y * stride..y * stride + cols * 4];
                    let out = &mut self.sums[(y / scale) * w..][..w];
                    for (o, cell) in out.iter_mut().zip(row.chunks_exact(scale * 4)) {
                        *o += cell
                            .chunks_exact(4)
                            .map(|p| luma(p[0] as u32, p[1] as u32, p[2] as u32))
                            .sum::<u32>();
                    }
                }
            }
        }

        // average each cell, then a 3x3 box blur (edge pixels average only
        // their existing neighbours) to quiet the sensor noise
        let area = (scale * scale) as f32;
        for (t, &s) in self.tmp.iter_mut().zip(&self.sums) {
            *t = s as f32 / area;
        }
        // horizontal pass into sums-sized scratch: reuse `gray`
        for y in 0..h {
            let src = &self.tmp[y * w..(y + 1) * w];
            let dst = &mut self.gray[y * w..(y + 1) * w];
            for x in 0..w {
                let lo = x.saturating_sub(1);
                let hi = (x + 1).min(w - 1);
                dst[x] = src[lo..=hi].iter().sum();
            }
        }
        // vertical pass back into `tmp`, then normalise by neighbour count
        for y in 0..h {
            let (lo, hi) = (y.saturating_sub(1), (y + 1).min(h - 1));
            let rows_n = (hi - lo + 1) as f32;
            let out = &mut self.tmp[y * w..(y + 1) * w];
            out.fill(0.0);
            for yy in lo..=hi {
                for (o, &g) in out.iter_mut().zip(&self.gray[yy * w..(yy + 1) * w]) {
                    *o += g;
                }
            }
            for (x, o) in out.iter_mut().enumerate() {
                let cols_n = ((x + 1).min(w - 1) - x.saturating_sub(1) + 1) as f32;
                *o /= rows_n * cols_n;
            }
        }
        std::mem::swap(&mut self.gray, &mut self.tmp);
        Ok(())
    }

    /// Begins learning the scene from the current frame.
    fn start(&mut self) {
        self.mean.clear();
        self.mean.extend_from_slice(&self.gray);
        self.vari.clear();
        self.vari.resize(self.gray.len(), INIT_VARIANCE);
        self.learned = true;
        self.settle_left = self.settle;
        self.scores = vec![0.0; self.window.max(1)];
        self.next = 0;
    }

    fn learn_all(&mut self, alpha: f32) {
        for ((&g, m), v) in self.gray.iter().zip(self.mean.iter_mut()).zip(self.vari.iter_mut()) {
            learn(m, v, g, alpha);
        }
    }

    /// Drops changed pixels with fewer than two changed neighbours.
    fn despeckle(&mut self) {
        let (w, h) = (self.w, self.h);
        for y in 0..h {
            let (lo, hi) = (y.saturating_sub(1), (y + 1).min(h - 1));
            for x in 0..w {
                let i = y * w + x;
                if !self.changed[i] {
                    self.keep[i] = false;
                    continue;
                }
                let (xl, xh) = (x.saturating_sub(1), (x + 1).min(w - 1));
                let mut nb = 0;
                for yy in lo..=hi {
                    nb += self.changed[yy * w + xl..=yy * w + xh].iter().filter(|&&c| c).count();
                }
                self.keep[i] = nb - 1 >= 2;
            }
        }
        std::mem::swap(&mut self.changed, &mut self.keep);
    }

    /// Pixel count of the largest 8-connected group of changed pixels.
    /// Clears `self.changed`.
    fn largest_blob(&mut self) -> usize {
        let (w, h) = (self.w, self.h);
        let mut largest = 0;
        for start in 0..self.changed.len() {
            if !self.changed[start] {
                continue;
            }
            self.changed[start] = false;
            self.stack.clear();
            self.stack.push(start as u32);
            let mut size = 0;
            while let Some(i) = self.stack.pop() {
                let i = i as usize;
                size += 1;
                let (x, y) = (i % w, i / w);
                for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                    for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                        let j = yy * w + xx;
                        if self.changed[j] {
                            self.changed[j] = false;
                            self.stack.push(j as u32);
                        }
                    }
                }
            }
            largest = largest.max(size);
        }
        largest
    }

    /// Records a frame score and returns the `persist`-th largest of the last
    /// `window` scores.
    fn push(&mut self, s: f64) -> f64 {
        if self.scores.is_empty() {
            return s;
        }
        self.scores[self.next] = s;
        self.next = (self.next + 1) % self.scores.len();
        let mut sorted = self.scores.clone();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let p = self.persist.max(1).min(sorted.len());
        sorted[p - 1]
    }
}

fn learn(mean: &mut f32, vari: &mut f32, g: f32, alpha: f32) {
    let diff = g - *mean;
    *mean += alpha * diff;
    *vari = (*vari + alpha * (diff * diff - *vari)).max(MIN_VARIANCE);
}

/// Brightness (0 to 255) of 8-bit R, G, B values (BT.601).
fn luma(r: u32, g: u32, b: u32) -> u32 {
    (19595 * r + 38470 * g + 7471 * b + (1 << 15)) >> 16
}

pub mod ffi;
