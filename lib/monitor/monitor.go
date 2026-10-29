// Package monitor counts what the camera loop does so it can be reported.
package monitor

import (
	"fmt"
	"sync/atomic"
	"time"
)

// Monitor records frames and alarms. It is safe for concurrent use.
type Monitor struct {
	camera    string
	threshold float64
	cooldown  time.Duration
	started   time.Time

	frames, alarms       atomic.Int64
	lastFrame, lastAlarm atomic.Int64 // unix nanos, 0 if none yet
}

// New starts a monitor. camera is the device id ("" for the default camera);
// threshold and cooldown are only shown in the report.
func New(camera string, threshold float64, cooldown time.Duration) *Monitor {
	return &Monitor{camera: camera, threshold: threshold, cooldown: cooldown, started: time.Now()}
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
	return fmt.Sprintf("Camera\n  device: %s\n  frames: %d (%.1f fps average)\n  last frame: %s\n  alarms sent: %d\n  last alarm: %s\n  alarm threshold: %g, cooldown %s",
		camera, frames, fps, ago(fromNanos(m.lastFrame.Load())), m.alarms.Load(), ago(m.LastAlarm()), m.threshold, m.cooldown)
}
