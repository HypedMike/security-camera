//! The real camera, through nokhwa: V4L2 on Linux, AVFoundation on macOS.

use crate::{Device, Frame, Source};
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
    Resolution,
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
            Device {
                id: i.index().as_string(),
                label,
            }
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
    let index = if id.is_empty() {
        CameraIndex::Index(0)
    } else {
        CameraIndex::String(id.to_string())
    };
    // YUYV is what the cameras deliver uncompressed; the closest format is
    // taken if the camera has no such mode
    let wanted = CameraFormat::new(Resolution::new(width, height), FrameFormat::YUYV, 30);
    let format = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Closest(wanted));
    let mut camera = Camera::new(index, format).map_err(|e| e.to_string())?;
    camera.open_stream().map_err(|e| e.to_string())?;
    Ok(Box::new(Nokhwa {
        camera,
        id: id.to_string(),
    }))
}

struct Nokhwa {
    camera: Camera,
    /// what the camera was opened with
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    id: String,
}

impl Source for Nokhwa {
    fn lock_exposure(&mut self) -> Result<String, String> {
        #[cfg(target_os = "linux")]
        {
            v4l_lock::lock(&self.id)
        }
        #[cfg(not(target_os = "linux"))]
        {
            use nokhwa::utils::{ControlValueSetter, KnownCameraControl};
            // AVFoundation: mode 0 is "locked", it keeps the current values
            let mut locked = Vec::new();
            let mut errors = Vec::new();
            for (control, name) in [
                (KnownCameraControl::Exposure, "exposure"),
                (KnownCameraControl::WhiteBalance, "white balance"),
            ] {
                match self
                    .camera
                    .set_camera_control(control, ControlValueSetter::EnumValue(0))
                {
                    Ok(()) => locked.push(name),
                    Err(e) => errors.push(format!("{name}: {e}")),
                }
            }
            if locked.is_empty() {
                return Err(errors.join("; "));
            }
            Ok(format!("locked the {}", locked.join(" and ")))
        }
    }

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
        Err(format!(
            "too many broken frames: {}",
            bad.unwrap_or_default()
        ))
    }
}

/// Locking the exposure on Linux goes straight to V4L2: nokhwa does not know
/// the control that turns auto exposure off.
#[cfg(target_os = "linux")]
mod v4l_lock {
    use v4l::Device;
    use v4l::control::{Control, Value};

    const AUTO_EXPOSURE: u32 = 0x009a_0901;
    const EXPOSURE_ABSOLUTE: u32 = 0x009a_0902;
    const AUTO_WHITE_BALANCE: u32 = 0x0098_090c;
    const AUTO_GAIN: u32 = 0x0098_0912;
    const WHITE_BALANCE_TEMPERATURE: u32 = 0x0098_091a;
    /// The value of AUTO_EXPOSURE that lets the program set the exposure.
    const EXPOSURE_MANUAL: i64 = 1;
    /// Controls that are on or off.
    const BOOLEANS: [u32; 2] = [AUTO_WHITE_BALANCE, AUTO_GAIN];

    /// The writes that freeze the picture, in the order to do them: the
    /// automatic mode goes off first, then the value it had settled on is set
    /// (a control is read-only while its automatic mode is on). `read` gives
    /// the value of a control, or `None` if the camera doesn't have it.
    pub(super) fn plan(read: impl Fn(u32) -> Option<i64>) -> Vec<(u32, i64)> {
        let mut writes = Vec::new();
        if read(AUTO_EXPOSURE).is_some() {
            writes.push((AUTO_EXPOSURE, EXPOSURE_MANUAL));
            if let Some(v) = read(EXPOSURE_ABSOLUTE) {
                writes.push((EXPOSURE_ABSOLUTE, v));
            }
        }
        if read(AUTO_WHITE_BALANCE).is_some() {
            writes.push((AUTO_WHITE_BALANCE, 0));
            if let Some(v) = read(WHITE_BALANCE_TEMPERATURE) {
                writes.push((WHITE_BALANCE_TEMPERATURE, v));
            }
        }
        if read(AUTO_GAIN).is_some() {
            writes.push((AUTO_GAIN, 0));
        }
        writes
    }

    fn open(id: &str) -> std::io::Result<Device> {
        match id.parse::<usize>() {
            Ok(n) => Device::new(n),
            Err(_) if id.is_empty() => Device::new(0),
            Err(_) => Device::with_path(id),
        }
    }

    fn name(id: u32) -> &'static str {
        match id {
            AUTO_EXPOSURE => "auto exposure",
            EXPOSURE_ABSOLUTE => "exposure time",
            AUTO_WHITE_BALANCE => "auto white balance",
            WHITE_BALANCE_TEMPERATURE => "white balance temperature",
            _ => "auto gain",
        }
    }

    pub(super) fn lock(id: &str) -> Result<String, String> {
        let dev = open(id).map_err(|e| e.to_string())?;
        let value = |id: u32| match dev.control(id).ok()?.value {
            Value::Integer(v) => Some(v),
            Value::Boolean(b) => Some(i64::from(b)),
            _ => None,
        };
        let writes = plan(value);
        if writes.is_empty() {
            return Err("the camera has no exposure controls".into());
        }
        let mut done = Vec::new();
        let mut errors = Vec::new();
        for (control, v) in writes {
            let value = if BOOLEANS.contains(&control) {
                Value::Boolean(v != 0)
            } else {
                Value::Integer(v)
            };
            match dev.set_control(Control { id: control, value }) {
                Ok(()) => done.push(name(control)),
                Err(e) => errors.push(format!("{}: {e}", name(control))),
            }
        }
        if done.is_empty() {
            return Err(errors.join("; "));
        }
        let mut what = format!("locked {}", done.join(", "));
        if !errors.is_empty() {
            what.push_str(&format!(" (failed: {})", errors.join("; ")));
        }
        Ok(what)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_automatic_modes_go_off_before_their_values_are_set() {
            // the camera of the user: no auto gain
            let read = |id: u32| match id {
                AUTO_EXPOSURE => Some(3),
                EXPOSURE_ABSOLUTE => Some(156),
                AUTO_WHITE_BALANCE => Some(1),
                WHITE_BALANCE_TEMPERATURE => Some(4600),
                _ => None,
            };
            assert_eq!(
                plan(read),
                [
                    (AUTO_EXPOSURE, 1),
                    (EXPOSURE_ABSOLUTE, 156),
                    (AUTO_WHITE_BALANCE, 0),
                    (WHITE_BALANCE_TEMPERATURE, 4600),
                ]
            );
        }

        #[test]
        fn a_camera_without_the_controls_gets_no_writes() {
            assert!(plan(|_| None).is_empty());
            // a value without its automatic mode is left alone
            assert!(plan(|id| (id == EXPOSURE_ABSOLUTE).then_some(10)).is_empty());
        }
    }
}
