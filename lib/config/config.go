// Package config holds the fixed settings of the security camera.
package config

import (
	"sync"
	"time"
)

// AlarmCooldown is the minimum time between two Telegram notifications.
const AlarmCooldown = 5 * time.Second

// When motion is detected the alarm message is sent at once. In the
// background the program then records the next AlarmGIFSpan of the camera at
// AlarmGIFFPS frames a second, saves them as a GIF in GIFDir and sends it as
// a reply to the alarm message when somebody reacts to it with a thumbs up.
//
// AlarmGIFSpan is how long the recording lasts, AlarmGIFFPS how many frames
// of it are kept every second.
const (
	AlarmGIFSpan = 5 * time.Second
	AlarmGIFFPS  = 2
)

// HDGIFWidth is the largest width of the GIF frames: the full 640x480.
const HDGIFWidth = 640

// HDGIFDither smooths the GIF colours, at the price of a file up to about
// 3 times bigger.
const HDGIFDither = true

// GIFDir is the directory the alarm GIFs are saved in.
const GIFDir = "gifs"

// GIFStorageLimit is the largest total size in bytes of the saved GIFs
// (1 GB). Past it the oldest GIFs are deleted.
const GIFStorageLimit = 1 << 30

// DailyReportHour is the hour (local time, 0-23) the daily "i'm alive"
// message is sent at.
const DailyReportHour = 9

// NightStartHour is the hour (local time, 0-23) the night starts at: the
// daily message counts the alarms since then on the previous evening.
const NightStartHour = 20

// The system alarms warn on Telegram when the machine is in trouble. The
// machine is checked every SystemCheckInterval.
const SystemCheckInterval = time.Minute

// DiskAlertPercent is the disk usage (0 to 100) from which the disk counts as
// almost full.
const DiskAlertPercent = 90

// CPUAlertPercent is the CPU usage (0 to 100) that is a problem when it lasts
// for CPUAlertChecks checks in a row (5 minutes): short peaks are normal.
const (
	CPUAlertPercent = 90
	CPUAlertChecks  = 5
)

// TempAlertC is the temperature in °C from which the machine is too hot.
const TempAlertC = 80

// OfflineAlertChecks is how many failed connection checks in a row mean the
// connection is lost. The alarm is sent when it comes back.
const OfflineAlertChecks = 2

// The Telegram mini app (MINI_APP_URL) shows the live view and the motion
// graph. It listens on MiniAppAddr, on this machine only: a tunnel such as
// Tailscale Funnel makes it reachable over HTTPS.
const MiniAppAddr = "127.0.0.1:8081"

// MiniAppFPS is how many video frames a second are sent to the mini app,
// MiniAppJPEGQuality their JPEG quality (1 to 100).
const (
	MiniAppFPS         = 5
	MiniAppJPEGQuality = 60
)

// MiniAppMaxViewers is how many video streams can be open at once.
const MiniAppMaxViewers = 3

// MiniAppMaxStream is the longest one video stream is kept open.
const MiniAppMaxStream = 15 * time.Minute

// MiniAppAuthMaxAge is how old the signed data Telegram gives the app when it
// opens may be: past that the app has to be opened again.
const MiniAppAuthMaxAge = 24 * time.Hour

// DefaultLatitude and DefaultLongitude are the coordinates of Lucca, used for
// the weather in the daily message when WEATHER_LATITUDE and
// WEATHER_LONGITUDE are not set.
const (
	DefaultLatitude  = 43.8430
	DefaultLongitude = 10.5027
)

// CameraChoiceTimeout is how long the camera prompt waits before using the first camera.
const CameraChoiceTimeout = 5 * time.Second

// EnvFile is the file the settings are loaded from and saved to.
const EnvFile = ".env"

// StreamHistory is how many past frames the stack keeps by default (n).
const StreamHistory = 300

// DefaultDetector is the motion detector used when DETECTOR is not set.
const DefaultDetector = "background"

// BackgroundThreshold is the default alarm threshold of the background
// detector: the area of the largest moving blob as a fraction of the frame.
const BackgroundThreshold = 0.003

// DifferenceThreshold is the default alarm threshold of the difference
// detector: the average change between consecutive frames.
const DifferenceThreshold = 0.05

// DetectionFrames is how many of the latest frames the difference detector
// computes the difference on by default (t).
const DetectionFrames = 30

// GraphAddr is the address the live graph is served on.
const GraphAddr = ":8080"

// SnapJPEGQuality is the JPEG quality of the /snap photos.
const SnapJPEGQuality = 85

// UpdateScript is the script /update runs to pull and rebuild the program.
const UpdateScript = "./update"

// UpdateTimeout is how long the update script may run.
const UpdateTimeout = 10 * time.Minute

// Values are the fixed settings above that /edit_config can change while the
// program runs.
type Values struct {
	AlarmGIFSpan time.Duration
	AlarmGIFFPS  int

	HDGIFWidth  int
	HDGIFDither bool

	SnapJPEGQuality int
}

// Live holds the current Values. It is safe for concurrent use.
type Live struct {
	mu sync.RWMutex
	v  Values
}

// NewLive returns a Live that starts at the constants above.
func NewLive() *Live {
	return &Live{v: Values{
		AlarmGIFSpan:    AlarmGIFSpan,
		AlarmGIFFPS:     AlarmGIFFPS,
		HDGIFWidth:      HDGIFWidth,
		HDGIFDither:     HDGIFDither,
		SnapJPEGQuality: SnapJPEGQuality,
	}}
}

// Get returns the current values.
func (l *Live) Get() Values {
	l.mu.RLock()
	defer l.mu.RUnlock()
	return l.v
}

// Set replaces the current values. The caller checks that they are valid.
func (l *Live) Set(v Values) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.v = v
}
