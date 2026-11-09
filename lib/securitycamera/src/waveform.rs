//! Draws the motion score over the time of an alarm clip as a strip along the
//! bottom of a GIF frame: the waveform of the movement, the alarm threshold as
//! a red line, and a white cursor on the moment of the frame.
//!
//! Only plain palette colours are used (see the `gif` crate), so the lines
//! come out sharp after the quantization.

use std::time::Duration;

use gif::RgbaImage;

/// Smallest picture that gets the strip: below it the strip would hide it.
const MIN_HEIGHT: usize = 48;
/// The strip is this fraction of the picture high (1/5), at least [`MIN_STRIP`].
const STRIP_DIVISOR: usize = 5;
const MIN_STRIP: usize = 24;

const LINE: [u8; 3] = [0x00, 0xff, 0x00];
const FILL: [u8; 3] = [0x00, 0x66, 0x00];
const THRESHOLD: [u8; 3] = [0xff, 0x00, 0x00];
const CURSOR: [u8; 3] = [0xff, 0xff, 0xff];

/// The motion scores of a clip: each one with how long after the start of the
/// clip its camera frame was taken.
pub struct Trace<'a> {
    pub scores: &'a [(Duration, f64)],
    /// How long the clip lasts, the width of the strip.
    pub span: Duration,
    /// The score that sets off the alarm.
    pub threshold: f64,
}

/// Draws the strip on `img` with the cursor at `now`, the time of that frame
/// in the clip. Nothing is drawn if there are no scores or the picture is too
/// small.
pub fn overlay(img: &mut RgbaImage, trace: &Trace, now: Duration) {
    let (w, h) = (img.width, img.height);
    if trace.scores.is_empty() || w < 2 || h < MIN_HEIGHT || img.pix.len() != w * h * 4 || trace.span.is_zero() {
        return;
    }
    let strip = (h / STRIP_DIVISOR).max(MIN_STRIP);
    let y0 = h - strip;

    // darken the picture behind the strip so the lines can be seen on anything
    for px in img.pix[y0 * w * 4..].chunks_exact_mut(4) {
        for c in &mut px[..3] {
            *c = (u16::from(*c) * 2 / 5) as u8;
        }
    }

    let max = trace.scores.iter().map(|s| s.1).fold(trace.threshold, f64::max) * 1.1;
    let max = if max.is_finite() && max > 0.0 { max } else { 1.0 };
    // row of a value inside the strip, with one pixel of margin
    let row = |v: f64| -> usize {
        let v = if v.is_finite() { v.clamp(0.0, max) } else { 0.0 };
        y0 + strip - 2 - ((strip - 3) as f64 * v / max).round() as usize
    };
    let col = |t: Duration| -> usize { ((w - 1) as f64 * (t.as_secs_f64() / trace.span.as_secs_f64()).clamp(0.0, 1.0)).round() as usize };

    // the waveform: the highest score that falls in each column, the gaps
    // between the camera frames filled by drawing straight lines
    let mut top = vec![usize::MAX; w];
    let mut prev: Option<(usize, usize)> = None;
    for &(t, v) in trace.scores {
        let (x, y) = (col(t), row(v));
        match prev {
            Some((px, py)) if x > px => {
                for (i, t) in top[px..=x].iter_mut().enumerate() {
                    let f = i as f64 / (x - px) as f64;
                    *t = (*t).min((py as f64 + (y as f64 - py as f64) * f).round() as usize);
                }
            }
            _ => top[x] = top[x].min(y),
        }
        prev = Some((x, y));
    }
    let bottom = y0 + strip - 2;
    let mut last = usize::MAX;
    for (x, &y) in top.iter().enumerate() {
        if y == usize::MAX {
            continue;
        }
        // join this column to the previous one so steep edges have no holes
        let from = if last == usize::MAX { y } else { last.min(y) };
        let to = if last == usize::MAX { y } else { last.max(y) };
        for yy in y..=bottom {
            set(img, x, yy, FILL);
        }
        for yy in from..=to {
            set(img, x, yy, LINE);
        }
        last = y;
    }

    // the threshold, dashed
    let ty = row(trace.threshold);
    for x in (0..w).filter(|x| x % 6 < 4) {
        set(img, x, ty, THRESHOLD);
    }

    // where the frame is in time
    let cx = col(now);
    for y in y0..h {
        set(img, cx, y, CURSOR);
    }
}

fn set(img: &mut RgbaImage, x: usize, y: usize, c: [u8; 3]) {
    if x < img.width && y < img.height {
        img.pix[(y * img.width + x) * 4..][..4].copy_from_slice(&[c[0], c[1], c[2], 0xff]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: usize, h: usize) -> RgbaImage {
        RgbaImage { width: w, height: h, pix: [200, 200, 200, 255].repeat(w * h) }
    }

    fn px(img: &RgbaImage, x: usize, y: usize) -> [u8; 3] {
        let p = &img.pix[(y * img.width + x) * 4..];
        [p[0], p[1], p[2]]
    }

    /// How high the strip is on a picture `h` high.
    fn strip(h: usize) -> usize {
        (h / STRIP_DIVISOR).max(MIN_STRIP)
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn trace(scores: &[(Duration, f64)]) -> Trace<'_> {
        Trace { scores, span: ms(1000), threshold: 0.5 }
    }

    #[test]
    fn the_strip_is_at_the_bottom_and_the_rest_is_untouched() {
        let mut img = picture(100, 100);
        let scores = [(ms(0), 0.0), (ms(500), 1.0), (ms(1000), 0.0)];
        overlay(&mut img, &trace(&scores), ms(0));
        let y0 = 100 - strip(100);
        assert_eq!(px(&img, 50, y0 - 1), [200, 200, 200], "above the strip");
        assert_eq!(px(&img, 70, y0), [80, 80, 80], "the strip darkens the picture");
    }

    #[test]
    fn the_peak_is_higher_than_the_quiet_and_drawn_in_the_line_colour() {
        let mut img = picture(101, 100);
        let scores = [(ms(0), 0.0), (ms(500), 1.0), (ms(1000), 0.0)];
        overlay(&mut img, &trace(&scores), ms(1000));
        let first_line = |x: usize| (0..100).find(|&y| px(&img, x, y) == LINE).expect("a line pixel");
        assert!(first_line(50) < first_line(10), "the middle of the clip is the peak");
        assert!(first_line(50) <= 100 - strip(100) + 3, "the peak reaches the top of the strip");
    }

    #[test]
    fn the_cursor_follows_the_frame_and_the_threshold_is_red() {
        let mut img = picture(101, 100);
        let scores = [(ms(0), 0.0), (ms(1000), 1.0)];
        overlay(&mut img, &trace(&scores), ms(250));
        assert_eq!(px(&img, 25, 99), CURSOR);
        assert_eq!(px(&img, 25, 100 - strip(100)), CURSOR);
        assert!((0..100).any(|y| px(&img, 0, y) == THRESHOLD), "the threshold line starts at the left edge");
    }

    #[test]
    fn a_picture_without_scores_or_too_small_stays_as_it_is() {
        let mut img = picture(100, 100);
        overlay(&mut img, &trace(&[]), ms(0));
        assert_eq!(img, picture(100, 100));
        let mut small = picture(100, MIN_HEIGHT - 1);
        overlay(&mut small, &trace(&[(ms(0), 1.0)]), ms(0));
        assert_eq!(small, picture(100, MIN_HEIGHT - 1));
    }

    #[test]
    fn odd_scores_do_not_panic() {
        let mut img = picture(64, 64);
        let scores = [(ms(0), f64::NAN), (ms(10), f64::INFINITY), (ms(5000), -1.0), (ms(20), 0.0)];
        overlay(&mut img, &trace(&scores), ms(9999));
        let mut flat = picture(64, 64);
        overlay(&mut flat, &Trace { scores: &[(ms(0), 0.0)], span: ms(1000), threshold: 0.0 }, ms(0));
    }
}
