//! Ignore areas: rectangles of the picture where motion does not count (a
//! tree in the wind, a street, a screen). They are drawn in the mini app and
//! saved in settings.json.
//!
//! Only the detection ignores them: the live view, the GIFs and /snap still
//! show the whole picture.

use serde::{Deserialize, Serialize};

/// How many ignore areas there can be.
pub const MAX_AREAS: usize = 20;

/// The grey the ignored pixels are painted with before detection: a constant
/// colour never looks like motion.
const FILL: [u8; 4] = [128, 128, 128, 255];

/// A rectangle as fractions of the picture: `x` and `y` are its top left
/// corner (0 is the left/top edge, 1 the right/bottom edge), `width` and
/// `height` its size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Area {
    /// Why this area can't be used, if it can't.
    pub fn validate(&self) -> Result<(), String> {
        let ok = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        if !(ok(self.x) && ok(self.y) && ok(self.width) && ok(self.height)) {
            return Err("ignore_areas: x, y, width and height must be between 0 and 1".into());
        }
        if self.width <= 0.0 || self.height <= 0.0 {
            return Err("ignore_areas: width and height must be more than 0".into());
        }
        // a hair of slack for the rounding of the browser
        if self.x + self.width > 1.0 + 1e-6 || self.y + self.height > 1.0 + 1e-6 {
            return Err("ignore_areas: the area goes out of the picture".into());
        }
        Ok(())
    }

    /// The pixels of a `width` x `height` picture it covers, as
    /// `(left, top, right, bottom)` with right and bottom excluded.
    fn pixels(&self, width: usize, height: usize) -> (usize, usize, usize, usize) {
        let edge = |f: f64, size: usize| ((f * size as f64).round().max(0.0) as usize).min(size);
        (
            edge(self.x, width),
            edge(self.y, height),
            edge(self.x + self.width, width),
            edge(self.y + self.height, height),
        )
    }
}

/// Why these areas can't be used, if they can't.
pub fn validate(areas: &[Area]) -> Result<(), String> {
    if areas.len() > MAX_AREAS {
        return Err(format!("ignore_areas: at most {MAX_AREAS} areas"));
    }
    areas.iter().try_for_each(Area::validate)
}

/// Which pixels of a `width` x `height` picture are inside the areas.
/// `None` when there is nothing to ignore.
pub fn mask(areas: &[Area], width: usize, height: usize) -> Option<Vec<bool>> {
    if areas.is_empty() || width == 0 || height == 0 {
        return None;
    }
    let mut mask = vec![false; width * height];
    for a in areas {
        let (left, top, right, bottom) = a.pixels(width, height);
        for y in top..bottom {
            mask[y * width + left..y * width + right].fill(true);
        }
    }
    Some(mask)
}

/// Paints the areas of the RGBA `pixels` grey.
pub fn paint(pixels: &mut [u8], width: usize, height: usize, areas: &[Area]) {
    if pixels.len() < width * height * 4 {
        return;
    }
    for a in areas {
        let (left, top, right, bottom) = a.pixels(width, height);
        for y in top..bottom {
            for px in pixels[(y * width + left) * 4..(y * width + right) * 4].chunks_exact_mut(4) {
                px.copy_from_slice(&FILL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: f64, y: f64, width: f64, height: f64) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn checks_the_areas() {
        assert!(validate(&[]).is_ok());
        assert!(validate(&[area(0.0, 0.0, 1.0, 1.0)]).is_ok());
        assert!(validate(&[area(0.5, 0.5, 0.5, 0.5)]).is_ok());
        for bad in [
            area(-0.1, 0.0, 0.5, 0.5),
            area(0.0, 0.0, 0.0, 0.5),
            area(0.0, 0.0, 0.5, 0.0),
            area(0.6, 0.0, 0.5, 0.5),
            area(0.0, 0.6, 0.5, 0.5),
            area(f64::NAN, 0.0, 0.5, 0.5),
            area(0.0, 0.0, 2.0, 0.5),
        ] {
            assert!(validate(&[bad]).is_err(), "{bad:?} accepted");
        }
        assert!(validate(&vec![area(0.0, 0.0, 0.1, 0.1); MAX_AREAS]).is_ok());
        assert!(validate(&vec![area(0.0, 0.0, 0.1, 0.1); MAX_AREAS + 1]).is_err());
    }

    #[test]
    fn the_mask_covers_the_area() {
        assert_eq!(mask(&[], 4, 4), None);
        // the right half of a 4x2 picture
        let m = mask(&[area(0.5, 0.0, 0.5, 1.0)], 4, 2).unwrap();
        assert_eq!(m, [false, false, true, true, false, false, true, true]);
        // two areas add up
        let m = mask(
            &[area(0.0, 0.0, 0.25, 0.5), area(0.75, 0.5, 0.25, 0.5)],
            4,
            2,
        )
        .unwrap();
        assert_eq!(m, [true, false, false, false, false, false, false, true]);
    }

    #[test]
    fn paints_only_the_area() {
        let mut px = [10, 20, 30, 40].repeat(4); // 2x2
        paint(&mut px, 2, 2, &[area(0.5, 0.5, 0.5, 0.5)]);
        assert_eq!(&px[..12], &[10, 20, 30, 40].repeat(3)[..]);
        assert_eq!(&px[12..], &FILL);
        // a buffer that is too short is left alone
        let mut short = vec![1u8; 4];
        paint(&mut short, 2, 2, &[area(0.0, 0.0, 1.0, 1.0)]);
        assert_eq!(short, [1, 1, 1, 1]);
    }
}
