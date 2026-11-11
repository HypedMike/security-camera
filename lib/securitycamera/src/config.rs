//! The fixed settings of the security camera, and the few of them that
//! `/edit_config` can change while the program runs ([`Live`]).

use std::sync::RwLock;
use std::time::Duration;

use crate::util::lock_rw_read;

/// The minimum time between two Telegram notifications.
pub const ALARM_COOLDOWN: Duration = Duration::from_secs(5);

// When motion is detected the alarm message is sent at once. In the
// background the program then records the next ALARM_GIF_SPAN of the camera
// at ALARM_GIF_FPS frames a second, saves them as a GIF in GIF_DIR and sends
// it as a reply to the alarm message when somebody reacts to it.
pub const ALARM_GIF_SPAN: Duration = Duration::from_secs(5);
pub const ALARM_GIF_FPS: u32 = 2;
/// How much of the camera from before the alarm goes in the GIF: the program
/// keeps this much in memory all the time.
pub const ALARM_GIF_PRE: Duration = Duration::from_secs(2);

/// The largest width of the GIF frames: the full 640x480.
pub const HD_GIF_WIDTH: usize = 640;

/// Smooths the GIF colours, at the price of a file up to about 3 times bigger.
pub const HD_GIF_DITHER: bool = true;

/// The directory the alarm GIFs are saved in.
pub const GIF_DIR: &str = "gifs";

/// The largest total size in bytes of the saved GIFs (1 GB). Past it the
/// oldest GIFs are deleted.
pub const GIF_STORAGE_LIMIT: u64 = 1 << 30;

/// The hour (local time, 0-23) the daily "i'm alive" message is sent at.
pub const DAILY_REPORT_HOUR: u32 = 9;

/// The hour (local time, 0-23) the night starts at: the daily message counts
/// the alarms since then on the previous evening.
pub const NIGHT_START_HOUR: u32 = 20;

/// The system alarms warn on Telegram when the machine is in trouble. The
/// machine is checked every SYSTEM_CHECK_INTERVAL.
pub const SYSTEM_CHECK_INTERVAL: Duration = Duration::from_secs(60);
/// The disk usage (0 to 100) from which the disk counts as almost full.
pub const DISK_ALERT_PERCENT: f64 = 90.0;
/// The CPU usage (0 to 100) that is a problem when it lasts for
/// CPU_ALERT_CHECKS checks in a row (5 minutes): short peaks are normal.
pub const CPU_ALERT_PERCENT: f64 = 90.0;
pub const CPU_ALERT_CHECKS: usize = 5;
/// The temperature in °C from which the machine is too hot.
pub const TEMP_ALERT_C: f64 = 80.0;
/// How many failed connection checks in a row mean the connection is lost.
/// The alarm is sent when it comes back.
pub const OFFLINE_ALERT_CHECKS: usize = 2;

/// The Telegram mini app (MINI_APP_URL) shows the live view and the motion
/// graph. It listens on MINI_APP_ADDR, on this machine only: a tunnel such as
/// Tailscale Funnel makes it reachable over HTTPS.
pub const MINI_APP_ADDR: &str = "127.0.0.1:8081";
/// How many video frames a second are sent to the mini app, and their JPEG
/// quality (1 to 100).
pub const MINI_APP_FPS: u32 = 5;
pub const MINI_APP_JPEG_QUALITY: u8 = 60;
/// How many video streams can be open at once.
pub const MINI_APP_MAX_VIEWERS: usize = 3;
/// The longest one video stream is kept open.
pub const MINI_APP_MAX_STREAM: Duration = Duration::from_secs(15 * 60);
/// How old the signed data Telegram gives the app when it opens may be: past
/// that the app has to be opened again.
pub const MINI_APP_AUTH_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The coordinates of Lucca, used for the weather in the daily message when
/// WEATHER_LATITUDE and WEATHER_LONGITUDE are not set.
pub const DEFAULT_LATITUDE: f64 = 43.8430;
pub const DEFAULT_LONGITUDE: f64 = 10.5027;

/// How long the camera prompt waits before using the first camera.
pub const CAMERA_CHOICE_TIMEOUT: Duration = Duration::from_secs(5);

/// The file the settings are loaded from and saved to.
pub const ENV_FILE: &str = ".env";

/// The JSON file /edit_config saves the settings in. When it exists at
/// startup its values override the defaults (constants and .env);
/// /reset_config deletes it.
pub const SETTINGS_FILE: &str = "settings.json";

/// How many past frames the stack keeps by default (n).
pub const STREAM_HISTORY: usize = 300;

/// The motion detector used when DETECTOR is not set.
pub const DEFAULT_DETECTOR: &str = "background";

/// The default alarm threshold of the background detector: the area of the
/// largest moving blob as a fraction of the frame.
pub const BACKGROUND_THRESHOLD: f64 = 0.003;

/// The default alarm threshold of the difference detector: the average
/// change between consecutive frames.
pub const DIFFERENCE_THRESHOLD: f64 = 0.05;

/// How many of the latest frames the difference detector computes the
/// difference on by default (t).
pub const DETECTION_FRAMES: usize = 30;

/// The address the live graph is served on.
pub const GRAPH_ADDR: &str = "0.0.0.0:8080";

/// The JPEG quality of the /snap photos.
pub const SNAP_JPEG_QUALITY: u8 = 85;

/// The brightness of the picture, in percent: 100 leaves the camera as it is,
/// less darkens it and more brightens it. The mini app changes it in
/// BRIGHTNESS_STEP steps between BRIGHTNESS_MIN and BRIGHTNESS_MAX.
pub const BRIGHTNESS: u32 = 100;
pub const BRIGHTNESS_MIN: u32 = 20;
pub const BRIGHTNESS_MAX: u32 = 400;
pub const BRIGHTNESS_STEP: u32 = 10;

/// The script /update runs to pull and rebuild the program.
pub const UPDATE_SCRIPT: &str = "./update";

/// How long the update script may run.
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The fixed settings above that /edit_config can change while the program
/// runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Values {
    pub alarm_gif_span: Duration,
    pub alarm_gif_fps: u32,
    pub hd_gif_width: usize,
    pub hd_gif_dither: bool,
    pub snap_jpeg_quality: u8,
    pub brightness: u32,
}

impl Default for Values {
    /// The constants above.
    fn default() -> Self {
        Values {
            alarm_gif_span: ALARM_GIF_SPAN,
            alarm_gif_fps: ALARM_GIF_FPS,
            hd_gif_width: HD_GIF_WIDTH,
            hd_gif_dither: HD_GIF_DITHER,
            snap_jpeg_quality: SNAP_JPEG_QUALITY,
            brightness: BRIGHTNESS,
        }
    }
}

/// The current [`Values`]. It can be shared between threads.
#[derive(Default)]
pub struct Live {
    v: RwLock<Values>,
}

impl Live {
    /// Starts at the constants above.
    pub fn new() -> Live {
        Live::default()
    }

    pub fn get(&self) -> Values {
        *lock_rw_read(&self.v)
    }

    /// Replaces the current values. The caller checks that they are valid.
    pub fn set(&self, v: Values) {
        *self.v.write().unwrap_or_else(|e| e.into_inner()) = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_starts_at_the_constants_and_changes() {
        let live = Live::new();
        assert_eq!(live.get().alarm_gif_fps, ALARM_GIF_FPS);
        let v = Values {
            alarm_gif_fps: 4,
            ..live.get()
        };
        live.set(v);
        assert_eq!(live.get(), v);
    }
}
