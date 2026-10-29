// Package monitor counts what the camera loop does so it can be reported.
package monitor

import (
	"fmt"
	"sync"
	"sync/atomic"
	"time"
)

// Monitor records frames and alarms. It is safe for concurrent use.
type Monitor struct {
	camera   string
	cooldown time.Duration
	started  time.Time

	// mu guards the detector and the thresholds, one per detector
	mu         sync.Mutex
	detector   string
	thresholds map[string]float64

	paused atomic.Bool

	detectFrames, gifFrames atomic.Int64

	frames, alarms       atomic.Int64
	lastFrame, lastAlarm atomic.Int64 // unix nanos, 0 if none yet
}

// New starts a monitor. camera is the device id ("" for the default camera);
// detector is the name of the motion detector in use and thresholds the
// alarm threshold of each detector to start with; they and the cooldown are
// shown in the report.
func New(camera, detector string, thresholds map[string]float64, cooldown time.Duration) *Monitor {
	m := &Monitor{camera: camera, cooldown: cooldown, started: time.Now(), detector: detector, thresholds: map[string]float64{}}
	for name, t := range thresholds {
		m.thresholds[name] = t
	}
	return m
}

// Detector returns the name of the motion detector in use.
func (m *Monitor) Detector() string {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.detector
}

// SetDetector changes the motion detector in use. The caller checks that
// the name is a known detector.
func (m *Monitor) SetDetector(name string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.detector = name
}

// DetectionFrames returns how many of the latest frames the difference is computed on.
func (m *Monitor) DetectionFrames() int { return int(m.detectFrames.Load()) }

// SetDetectionFrames changes how many of the latest frames the difference is computed on.
func (m *Monitor) SetDetectionFrames(n int) { m.detectFrames.Store(int64(n)) }

// GIFFrames returns how many of the latest frames an alarm GIF contains.
func (m *Monitor) GIFFrames() int { return int(m.gifFrames.Load()) }

// SetGIFFrames changes how many of the latest frames an alarm GIF contains.
func (m *Monitor) SetGIFFrames(n int) { m.gifFrames.Store(int64(n)) }

// Threshold returns the motion score at or above which the detector in use
// sends an alarm.
func (m *Monitor) Threshold() float64 {
	return m.ThresholdOf(m.Detector())
}

// ThresholdOf returns the alarm threshold of the named detector.
func (m *Monitor) ThresholdOf(detector string) float64 {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.thresholds[detector]
}

// SetThreshold changes the alarm threshold of the detector in use.
func (m *Monitor) SetThreshold(t float64) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.thresholds[m.detector] = t
}

// SetPaused pauses or resumes the alarms.
func (m *Monitor) SetPaused(paused bool) {
	m.paused.Store(paused)
}

// Paused reports whether the alarms are paused.
func (m *Monitor) Paused() bool {
	return m.paused.Load()
}

// Frame records a frame captured at ts.
func (m *Monitor) Frame(ts time.Time) {
	m.frames.Add(1)
	m.lastFrame.Store(ts.UnixNano())
}

// Alarm records an alarm raised at ts.
func (m *Monitor) Alarm(ts time.Time) {
	m.alarms.Add(1)
	m.lastAlarm.Store(ts.UnixNano())
}

// LastAlarm returns when the last alarm was raised, or the zero time if none was.
func (m *Monitor) LastAlarm() time.Time {
	return fromNanos(m.lastAlarm.Load())
}

func fromNanos(n int64) time.Time {
	if n == 0 {
		return time.Time{}
	}
	return time.Unix(0, n)
}

func ago(t time.Time) string {
	if t.IsZero() {
		return "never"
	}
	return time.Since(t).Round(time.Second).String() + " ago"
}

// Report describes what the camera part of the program has been doing.
func (m *Monitor) Report() string {
	camera := m.camera
	if camera == "" {
		camera = "default"
	}
	frames := m.frames.Load()
	fps := 0.0
	if el := time.Since(m.started).Seconds(); el > 0 {
		fps = float64(frames) / el
	}
	state := "active"
	if m.Paused() {
		state = "paused"
	}
	return fmt.Sprintf("Camera\n  device: %s\n  frames: %d (%.1f fps average)\n  last frame: %s\n  alarms sent: %d\n  alarm state: %s\n  last alarm: %s\n  detector: %s\n  alarm threshold: %g, cooldown %s\n  detection frames: %d, gif frames: %d",
		camera, frames, fps, ago(fromNanos(m.lastFrame.Load())), m.alarms.Load(), state, ago(m.LastAlarm()), m.Detector(), m.Threshold(), m.cooldown, m.DetectionFrames(), m.GIFFrames())
}
