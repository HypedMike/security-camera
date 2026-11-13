//! The motion detectors: each turns a camera frame into a score.

use detection::Background;
use my_video::Frame;
use streaming::Ring;

use crate::ignore::{self, Area};

/// Gives the ignore areas in use, read for every frame.
pub type Ignore = Box<dyn Fn() -> Vec<Area> + Send>;

/// Names of the detectors, as used by the DETECTOR setting and /edit_config.
pub const NAME_BACKGROUND: &str = "background";
pub const NAME_DIFFERENCE: &str = "difference";

/// Turns each camera frame into a motion score: 0 means no motion, and the
/// higher the score the more motion. What the score measures depends on the
/// detector, so every detector has its own alarm threshold.
///
/// A detector may keep state between frames, so it is fed every frame in
/// order.
pub trait Detector: Send {
    /// Identifies the detector, e.g. [`NAME_BACKGROUND`].
    fn name(&self) -> &'static str;
    /// The motion score of `frame`. `history` holds the previous frames,
    /// oldest first; detectors that keep their own state ignore it.
    fn score(&mut self, frame: &Frame, history: &Ring) -> Result<f64, String>;
    /// Forgets the state learned so far, e.g. when the detector is switched
    /// back on after frames went by without it.
    fn reset(&mut self);
}

/// Detects motion against a learned model of the empty scene, see the
/// `detection` crate for how it works.
pub struct BackgroundDetector {
    model: Background,
    ignore: Option<Ignore>,
}

impl BackgroundDetector {
    pub fn new() -> Self {
        BackgroundDetector {
            model: Background::new(),
            ignore: None,
        }
    }

    /// Motion inside the areas `ignore` gives does not count.
    pub fn ignoring(ignore: impl Fn() -> Vec<Area> + Send + 'static) -> Self {
        BackgroundDetector {
            ignore: Some(Box::new(ignore)),
            ..Self::new()
        }
    }
}

impl Default for BackgroundDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector for BackgroundDetector {
    fn name(&self) -> &'static str {
        NAME_BACKGROUND
    }

    fn reset(&mut self) {
        self.model.reset();
    }

    fn score(&mut self, frame: &Frame, _: &Ring) -> Result<f64, String> {
        let (width, height) = (frame.width as usize, frame.height as usize);
        if width == 0 || height == 0 || frame.pixels.is_empty() {
            return Err("detection: frame is empty".into());
        }
        let areas = self.ignore.as_ref().map(|f| f()).unwrap_or_default();
        // the ignored part is painted a constant grey, which the model learns
        // as an empty scene
        let painted;
        let pixels = if areas.is_empty() {
            &frame.pixels
        } else {
            let mut copy = frame.pixels.clone();
            ignore::paint(&mut copy, width, height, &areas);
            painted = copy;
            &painted
        };
        self.model
            .score(detection::Frame::Rgba {
                data: pixels,
                width,
                height,
                stride: width * 4,
            })
            .map_err(|e| e.to_string())
    }
}

/// An image for [`difference`]: tightly packed RGBA, its width and height.
pub type Pixels<'a> = (&'a [u8], usize, usize);

/// How much a sequence of images differs at pixel level. It compares each
/// image with the next one and returns the average absolute per-channel (R,
/// G, B) difference across all pixels and pairs, normalized to [0, 1]: 0
/// means identical images, 1 means every channel flipped between black and
/// white. All images must have the same size.
pub fn difference(images: &[Pixels]) -> Result<f64, String> {
    difference_outside(images, None)
}

/// Like [`difference`], but the pixels `ignored` marks (`width * height`
/// flags) are left out of the average.
pub fn difference_outside(images: &[Pixels], ignored: Option<&[bool]>) -> Result<f64, String> {
    let Some(&(_, width, height)) = images.first().filter(|_| images.len() >= 2) else {
        return Err("detection: need at least 2 images".into());
    };
    if images
        .iter()
        .any(|&(data, w, h)| (w, h) != (width, height) || data.len() != w * h * 4)
    {
        return Err("detection: images have different bounds".into());
    }
    if width == 0 || height == 0 {
        return Err("detection: images are empty".into());
    }
    let ignored = ignored.filter(|m| m.len() == width * height);
    let counted = match ignored {
        Some(m) => m.iter().filter(|&&i| !i).count(),
        None => width * height,
    };
    if counted == 0 {
        // everything is ignored
        return Ok(0.0);
    }
    let total: f64 = images
        .windows(2)
        .map(|pair| pair_difference(pair[0].0, pair[1].0, counted, ignored))
        .sum();
    Ok(total / (images.len() - 1) as f64)
}

fn pair_difference(a: &[u8], b: &[u8], counted: usize, ignored: Option<&[bool]>) -> f64 {
    let diff = |(p, q): (&[u8], &[u8])| {
        u64::from(p[0].abs_diff(q[0]))
            + u64::from(p[1].abs_diff(q[1]))
            + u64::from(p[2].abs_diff(q[2]))
    };
    let pairs = a.chunks_exact(4).zip(b.chunks_exact(4));
    let sum: u64 = match ignored {
        None => pairs.map(diff).sum(),
        Some(m) => pairs
            .zip(m)
            .filter(|&(_, &i)| !i)
            .map(|(p, _)| diff(p))
            .sum(),
    };
    sum as f64 / (counted as f64 * 3.0 * 255.0)
}

/// The detector built on [`difference`]: the score is the average change
/// between consecutive frames over the latest frames.
pub struct FrameDifference {
    /// How many of the latest frames the difference is computed on, counting
    /// the current one. Less than 2 uses the whole history.
    frames: Box<dyn Fn() -> usize + Send>,
    ignore: Option<Ignore>,
}

impl FrameDifference {
    pub fn new(frames: impl Fn() -> usize + Send + 'static) -> Self {
        FrameDifference {
            frames: Box::new(frames),
            ignore: None,
        }
    }

    /// Changes inside the areas `ignore` gives do not count.
    pub fn ignoring(mut self, ignore: impl Fn() -> Vec<Area> + Send + 'static) -> Self {
        self.ignore = Some(Box::new(ignore));
        self
    }
}

impl Detector for FrameDifference {
    fn name(&self) -> &'static str {
        NAME_DIFFERENCE
    }

    fn reset(&mut self) {}

    fn score(&mut self, frame: &Frame, history: &Ring) -> Result<f64, String> {
        if history.is_empty() {
            return Ok(0.0);
        }
        let n = (self.frames)();
        let used = if n >= 2 && n - 1 < history.len() {
            n - 1
        } else {
            history.len()
        };
        let mut images: Vec<Pixels> = (history.len() - used..history.len())
            .filter_map(|i| history.frame(i))
            .collect();
        let (width, height) = (frame.width as usize, frame.height as usize);
        images.push((&frame.pixels, width, height));
        let areas = self.ignore.as_ref().map(|f| f()).unwrap_or_default();
        difference_outside(&images, ignore::mask(&areas, width, height).as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(v: u8, w: usize, h: usize) -> Vec<u8> {
        [v, v, v, 255].repeat(w * h)
    }

    fn frame(v: u8) -> Frame {
        Frame {
            width: 4,
            height: 4,
            pixels: solid(v, 4, 4),
        }
    }

    fn ring(values: &[u8]) -> Ring {
        let mut r = Ring::new(values.len().max(1));
        for &v in values {
            let px = solid(v, 4, 4);
            r.remember(&gif::Image::Rgba {
                data: &px,
                width: 4,
                height: 4,
                stride: 16,
            });
        }
        r
    }

    #[test]
    fn difference_of_images() {
        let (black, white) = (solid(0, 4, 4), solid(255, 4, 4));
        fn img(d: &[u8]) -> Pixels<'_> {
            (d, 4, 4)
        }
        assert_eq!(
            difference(&[img(&black), img(&black)]),
            Ok(0.0),
            "identical"
        );
        assert_eq!(
            difference(&[img(&black), img(&white)]),
            Ok(1.0),
            "black/white"
        );
        assert_eq!(
            difference(&[img(&black), img(&white), img(&white)]),
            Ok(0.5),
            "black/white/white"
        );
        assert!(difference(&[img(&black)]).is_err(), "a single image");
        assert!(difference(&[]).is_err());
        let small = solid(0, 2, 2);
        assert!(
            difference(&[img(&black), (&small, 2, 2)]).is_err(),
            "mismatched sizes"
        );
        assert!(difference(&[(&[], 0, 0), (&[], 0, 0)]).is_err(), "empty");
        assert!(
            difference(&[img(&black), (&black[..8], 4, 4)]).is_err(),
            "buffer too short"
        );
    }

    #[test]
    fn ignored_pixels_do_not_count() {
        // a 2x1 picture: the left pixel flips, the right one stays
        let a = [0, 0, 0, 255, 50, 50, 50, 255];
        let b = [255, 255, 255, 255, 50, 50, 50, 255];
        let imgs = [(&a[..], 2, 1), (&b[..], 2, 1)];
        assert_eq!(difference_outside(&imgs, None), Ok(0.5));
        assert_eq!(difference_outside(&imgs, Some(&[false, true])), Ok(1.0));
        assert_eq!(difference_outside(&imgs, Some(&[true, false])), Ok(0.0));
        assert_eq!(difference_outside(&imgs, Some(&[true, true])), Ok(0.0));
        // a mask of the wrong size is not used
        assert_eq!(difference_outside(&imgs, Some(&[true])), Ok(0.5));
    }

    #[test]
    fn frame_difference_skips_the_ignore_areas() {
        let history = ring(&[0, 0]);
        let whole = Area {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let right_half = Area {
            x: 0.5,
            width: 0.5,
            ..whole
        };
        assert_eq!(
            FrameDifference::new(|| 0).score(&frame(255), &history),
            Ok(0.5)
        );
        // all of the picture ignored: nothing can move
        let mut d = FrameDifference::new(|| 0).ignoring(move || vec![whole]);
        assert_eq!(d.score(&frame(255), &history), Ok(0.0));
        // half of it ignored: the other half still sees the change
        let mut d = FrameDifference::new(|| 0).ignoring(move || vec![right_half]);
        assert_eq!(d.score(&frame(255), &history), Ok(0.5));
    }

    #[test]
    fn the_background_detector_ignores_a_blinking_area() {
        let (w, h) = (64usize, 48usize);
        // a bright square that moves a little inside the top left corner
        let scene = |shift: Option<usize>| {
            let mut px = [100, 100, 100, 255].repeat(w * h);
            for y in 0..16 {
                for x in 0..16 {
                    if let Some(d) = shift {
                        px[(y * w + x + d) * 4..][..3].copy_from_slice(&[255, 255, 255]);
                    }
                }
            }
            Frame {
                width: w as u32,
                height: h as u32,
                pixels: px,
            }
        };
        let corner = Area {
            x: 0.0,
            y: 0.0,
            width: 0.3,
            height: 0.4,
        };
        let r = ring(&[]);
        let learn = |d: &mut BackgroundDetector| {
            for _ in 0..40 {
                d.score(&scene(None), &r).unwrap();
            }
        };
        let moving = |d: &mut BackgroundDetector| {
            (0..5)
                .map(|i| d.score(&scene(Some(i % 3)), &r).unwrap())
                .fold(0.0, f64::max)
        };
        let mut plain = BackgroundDetector::new();
        learn(&mut plain);
        assert!(moving(&mut plain) > 0.0);
        let mut ignoring = BackgroundDetector::ignoring(move || vec![corner]);
        learn(&mut ignoring);
        assert_eq!(moving(&mut ignoring), 0.0);
    }

    #[test]
    fn alpha_does_not_count() {
        let a = [10, 20, 30, 0].repeat(4);
        let b = [10, 20, 30, 255].repeat(4);
        assert_eq!(difference(&[(&a, 2, 2), (&b, 2, 2)]), Ok(0.0));
    }

    #[test]
    fn frame_difference_without_history_is_zero() {
        let mut d = FrameDifference::new(|| 30);
        assert_eq!(d.score(&frame(0), &ring(&[])), Ok(0.0));
        assert_eq!(d.name(), NAME_DIFFERENCE);
    }

    #[test]
    fn frame_difference_uses_the_latest_frames() {
        // history: black, black, white; current: white
        let history = ring(&[0, 0, 255]);
        // all of it: the pairs 0-0, 0-255 and 255-255 → (0+1+0)/3
        assert_eq!(
            FrameDifference::new(|| 0).score(&frame(255), &history),
            Ok(1.0 / 3.0)
        );
        // the last 2 frames: 255-255 → 0
        assert_eq!(
            FrameDifference::new(|| 2).score(&frame(255), &history),
            Ok(0.0)
        );
        // the last 3: 0-255, 255-255 → 0.5
        assert_eq!(
            FrameDifference::new(|| 3).score(&frame(255), &history),
            Ok(0.5)
        );
        // more than there is: all of it
        assert_eq!(
            FrameDifference::new(|| 99).score(&frame(255), &history),
            Ok(1.0 / 3.0)
        );
    }

    #[test]
    fn the_background_detector_scores_frames_and_forgets() {
        let mut d = BackgroundDetector::new();
        let r = ring(&[]);
        for _ in 0..40 {
            assert_eq!(
                d.score(&frame(100), &r),
                Ok(0.0),
                "a still scene is not motion"
            );
        }
        d.reset();
        assert_eq!(d.name(), NAME_BACKGROUND);
        assert!(
            d.score(&Frame::default(), &r).is_err(),
            "an empty frame is refused"
        );
    }
}
