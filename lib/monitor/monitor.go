// Package monitor counts what the camera loop does so it can be reported.
package monitor

import (
	"fmt"
	"math"
	"sync/atomic"
	"time"
)

// Monitor records frames and alarms. It is safe for concurrent use.
type Monitor struct {
	camera   string
	cooldown time.Duration
	started  time.Time

	threshold atomic.Uint64 // float64 bits
	paused    atomic.Bool

	frames, alarms       atomic.Int64
	lastFrame, lastAlarm atomic.Int64 // unix nanos, 0 if none yet
}

// New starts a monitor. camera is the device id ("" for the default camera);
// threshold is the alarm threshold to start with; it and the cooldown are
// shown in the report.
func New(camera string, threshold float64, cooldown time.Duration) *Monitor {
	m := &Monitor{camera: camera, cooldown: cooldown, started: time.Now()}
	m.SetThreshold(threshold)
	return m
}

// Threshold returns the frame difference at or above which an alarm is sent.
func (m *Monitor) Threshold() float64 {
	return math.Float64frombits(m.threshold.Load())
}

// SetThreshold changes the alarm threshold.
func (m *Monitor) SetThreshold(t float64) {
	m.threshold.Store(math.Float64bits(t))
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
	return fmt.Sprintf("Camera\n  device: %s\n  frames: %d (%.1f fps average)\n  last frame: %s\n  alarms sent: %d\n  alarm state: %s\n  last alarm: %s\n  alarm threshold: %g, cooldown %s",
		camera, frames, fps, ago(fromNanos(m.lastFrame.Load())), m.alarms.Load(), state, ago(m.LastAlarm()), m.Threshold(), m.cooldown)
}
