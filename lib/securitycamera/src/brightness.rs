//! Brightening and darkening the camera picture, in software: it works with
//! every camera, whether or not the driver has a brightness control.

use std::sync::Arc;

use my_video::Frame;

use crate::config::{BRIGHTNESS, Live};
use crate::stream::Source;

/// Scales the colour of every pixel of the RGBA `pixels` by `percent` / 100
/// (100 changes nothing, 200 doubles it), stopping at white. Alpha is kept.
pub fn apply(pixels: &mut [u8], percent: u32) {
    if percent == BRIGHTNESS {
        return;
    }
    let mut table = [0u8; 256];
    for (v, out) in table.iter_mut().enumerate() {
        *out = ((v as u32 * percent + 50) / 100).min(255) as u8;
    }
    for px in pixels.chunks_exact_mut(4) {
        px[0] = table[px[0] as usize];
        px[1] = table[px[1] as usize];
        px[2] = table[px[2] as usize];
    }
}

/// A source whose frames have the current brightness setting applied, before
/// anything else (the detection, the mini app, the GIFs, /snap) sees them.
pub struct Brightened<S> {
    pub inner: S,
    pub live: Arc<Live>,
}

impl<S: Source> Source for Brightened<S> {
    fn read(&mut self, frame: &mut Frame) -> Result<(), String> {
        self.inner.read(frame)?;
        apply(&mut frame.pixels, self.live.get().brightness);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Values;

    #[test]
    fn scales_the_colours_and_keeps_alpha() {
        let mut px = vec![10, 100, 200, 255, 0, 0, 0, 7];
        apply(&mut px, 150);
        assert_eq!(px, [15, 150, 255, 255, 0, 0, 0, 7]);
        let mut px = vec![10, 100, 200, 255];
        apply(&mut px, 50);
        assert_eq!(px, [5, 50, 100, 255]);
    }

    #[test]
    fn a_hundred_percent_changes_nothing() {
        let mut px: Vec<u8> = (0..=255).collect();
        let before = px.clone();
        apply(&mut px, 100);
        assert_eq!(px, before);
    }

    struct Gray;
    impl Source for Gray {
        fn read(&mut self, frame: &mut Frame) -> Result<(), String> {
            frame.width = 1;
            frame.height = 1;
            frame.pixels = vec![100, 100, 100, 255];
            Ok(())
        }
    }

    #[test]
    fn the_source_follows_the_setting_while_running() {
        let live = Arc::new(Live::new());
        let mut src = Brightened {
            inner: Gray,
            live: live.clone(),
        };
        let mut f = Frame::default();
        src.read(&mut f).unwrap();
        assert_eq!(f.pixels, [100, 100, 100, 255]);
        live.set(Values {
            brightness: 200,
            ..live.get()
        });
        src.read(&mut f).unwrap();
        assert_eq!(f.pixels, [200, 200, 200, 255]);
    }
}
