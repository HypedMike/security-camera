//! The real camera, through nokhwa: V4L2 on Linux, AVFoundation on macOS.

use crate::{Device, Frame, Source};
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
};
use nokhwa::{Camera, query};

/// Frames that can't be decoded (a camera now and then sends a broken MJPEG
/// one) are skipped, up to this many in a row.
const MAX_BAD_FRAMES: usize = 30;

pub fn devices() -> Vec<Device> {
    if let Err(e) = authorize() {
        eprintln!("camera: {e}");
        return vec![];
    }
    let infos = match query(ApiBackend::Auto) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("camera: can't list the cameras: {e}");
            return vec![];
        }
    };
    infos
        .iter()
        .map(|i| {
            // The index is not stable, so the label is made of what the
            // driver says about the camera: its name and where it is.
            let label = [i.human_name(), i.description().to_string(), i.misc()]
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(";");
            Device { id: i.index().as_string(), label }
        })
        .collect()
}

/// macOS asks the user to allow the camera, and nothing works before the
/// answer. Elsewhere there is nothing to ask.
fn authorize() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::sync::Once;
        use std::time::{Duration, Instant};
        static ASKED: Once = Once::new();
        ASKED.call_once(|| nokhwa::nokhwa_initialize(|_| {}));
        let deadline = Instant::now() + Duration::from_secs(60);
        while !nokhwa::nokhwa_check() {
            if Instant::now() > deadline {
                return Err("the camera is not allowed: grant it in System Settings > Privacy & Security > Camera".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}

pub fn open(id: &str, width: u32, height: u32) -> Result<Box<dyn Source>, String> {
    authorize()?;
    // an empty id is the default camera
    let index = if id.is_empty() { CameraIndex::Index(0) } else { CameraIndex::String(id.to_string()) };
    // YUYV is what the cameras deliver uncompressed; the closest format is
    // taken if the camera has no such mode
    let wanted = CameraFormat::new(Resolution::new(width, height), FrameFormat::YUYV, 30);
    let format = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Closest(wanted));
    let mut camera = Camera::new(index, format).map_err(|e| e.to_string())?;
    camera.open_stream().map_err(|e| e.to_string())?;
    Ok(Box::new(Nokhwa { camera }))
}

struct Nokhwa {
    camera: Camera,
}

impl Source for Nokhwa {
    fn read(&mut self, frame: &mut Frame) -> Result<(), String> {
        let mut bad = None;
        for _ in 0..MAX_BAD_FRAMES {
            let buffer = self.camera.frame().map_err(|e| e.to_string())?;
            let res = buffer.resolution();
            let len = res.width() as usize * res.height() as usize * 4;
            frame.pixels.resize(len, 0);
            match buffer.decode_image_to_buffer::<RgbAFormat>(&mut frame.pixels) {
                Ok(()) => {
                    frame.width = res.width();
                    frame.height = res.height();
                    return Ok(());
                }
                Err(e) => bad = Some(e.to_string()),
            }
        }
        Err(format!("too many broken frames: {}", bad.unwrap_or_default()))
    }
}
