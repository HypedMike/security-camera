package monitor

import (
	"strings"
	"testing"
	"time"
)

func TestMonitor(t *testing.T) {
	m := New("", 0.05, 30*time.Second)
	if !m.LastAlarm().IsZero() {
		t.Fatal("alarm before any was raised")
	}
	r := m.Report()
	for _, want := range []string{"device: default", "frames: 0", "last frame: never", "last alarm: never"} {
		if !strings.Contains(r, want) {
			t.Errorf("report misses %q:\n%s", want, r)
		}
	}

	now := time.Now()
	m.Frame(now)
	m.Frame(now)
	m.Alarm(now)
	if !m.LastAlarm().Equal(now) {
		t.Fatalf("LastAlarm = %v, want %v", m.LastAlarm(), now)
	}
	r = m.Report()
	for _, want := range []string{"frames: 2", "alarms sent: 1", "threshold: 0.05", "cooldown 30s"} {
		if !strings.Contains(r, want) {
			t.Errorf("report misses %q:\n%s", want, r)
		}
	}
}
