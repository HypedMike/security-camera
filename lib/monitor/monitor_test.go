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

func TestPauseAndThreshold(t *testing.T) {
	m := New("", 0.05, 30*time.Second)
	if m.Paused() || m.Threshold() != 0.05 {
		t.Fatalf("initial state: paused=%v threshold=%v", m.Paused(), m.Threshold())
	}
	m.SetPaused(true)
	m.SetThreshold(0.2)
	if !m.Paused() || m.Threshold() != 0.2 {
		t.Fatalf("after change: paused=%v threshold=%v", m.Paused(), m.Threshold())
	}
	r := m.Report()
	for _, want := range []string{"alarm state: paused", "threshold: 0.2"} {
		if !strings.Contains(r, want) {
			t.Errorf("report misses %q:\n%s", want, r)
		}
	}
	m.SetPaused(false)
	if m.Paused() {
		t.Error("still paused after resume")
	}
}
