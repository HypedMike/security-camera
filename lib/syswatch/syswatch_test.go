package syswatch

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"
)

var limits = Limits{DiskPercent: 90, CPUPercent: 90, CPUChecks: 3, TempC: 80, OfflineChecks: 2}

func at(min int) time.Time { return time.Date(2026, 11, 3, 14, min, 0, 0, time.UTC) }

func ok() Reading {
	return Reading{Disk: 50, CPU: 10, Temp: 40, HasDisk: true, HasCPU: true, HasTemp: true, Online: true}
}

func wantOne(t *testing.T, got []string, sub string) {
	t.Helper()
	if len(got) != 1 || !strings.Contains(got[0], sub) {
		t.Fatalf("got %q, want one message with %q", got, sub)
	}
}

func TestQuietWhenAllIsFine(t *testing.T) {
	w := New(limits)
	for i := 0; i < 5; i++ {
		if got := w.Update(at(i), ok()); len(got) != 0 {
			t.Fatalf("check %d: %q", i, got)
		}
	}
}

func TestDisk(t *testing.T) {
	w := New(limits)
	r := ok()
	r.Disk = 93
	wantOne(t, w.Update(at(0), r), "Disk almost full: 93%")
	if got := w.Update(at(1), r); len(got) != 0 {
		t.Errorf("repeated: %q", got)
	}
	r.Disk = 87 // below the limit, but within the hysteresis
	if got := w.Update(at(2), r); len(got) != 0 {
		t.Errorf("flapping: %q", got)
	}
	r.Disk = 80
	wantOne(t, w.Update(at(3), r), "Disk back to normal: 80%")
	if got := w.Update(at(4), r); len(got) != 0 {
		t.Errorf("after recovery: %q", got)
	}
}

func TestTemperature(t *testing.T) {
	w := New(limits)
	r := ok()
	r.Temp = 85
	wantOne(t, w.Update(at(0), r), "Temperature high: 85°C")
	r.Temp = 60
	wantOne(t, w.Update(at(1), r), "Temperature back to normal: 60°C")
}

func TestCPUNeedsToLast(t *testing.T) {
	w := New(limits)
	r := ok()
	r.CPU = 99
	for i := 0; i < 2; i++ {
		if got := w.Update(at(i), r); len(got) != 0 {
			t.Fatalf("peak %d reported: %q", i, got)
		}
	}
	r.CPU = 20 // the peak ends: the count starts again
	w.Update(at(2), r)
	r.CPU = 99
	for i := 3; i < 5; i++ {
		if got := w.Update(at(i), r); len(got) != 0 {
			t.Fatalf("second peak %d reported: %q", i, got)
		}
	}
	wantOne(t, w.Update(at(5), r), "CPU busy: 99% for 3 checks")
	if got := w.Update(at(6), r); len(got) != 0 {
		t.Errorf("repeated: %q", got)
	}
	r.CPU = 30
	wantOne(t, w.Update(at(7), r), "CPU back to normal")
}

func TestConnection(t *testing.T) {
	w := New(limits)
	off := ok()
	off.Online = false
	for i := 0; i < 5; i++ { // nothing can be said while offline
		if got := w.Update(at(i), off); len(got) != 0 {
			t.Fatalf("offline check %d: %q", i, got)
		}
	}
	wantOne(t, w.Update(at(10), ok()), "Connection back after 10m0s (lost at 14:00)")
	if got := w.Update(at(11), ok()); len(got) != 0 {
		t.Errorf("after recovery: %q", got)
	}
}

func TestConnectionHiccup(t *testing.T) {
	w := New(limits) // 2 failures needed
	off := ok()
	off.Online = false
	w.Update(at(0), off)
	if got := w.Update(at(1), ok()); len(got) != 0 {
		t.Errorf("single failure reported: %q", got)
	}
	w.Update(at(2), off) // the count must have started again
	if got := w.Update(at(3), ok()); len(got) != 0 {
		t.Errorf("single failure reported: %q", got)
	}
}

func TestUnreadValuesAreIgnored(t *testing.T) {
	w := New(limits)
	if got := w.Update(at(0), Reading{Online: true}); len(got) != 0 {
		t.Errorf("got %q", got)
	}
}

func TestRunRetriesUnsentMessages(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	checks := 0
	check := func(context.Context) Reading {
		checks++
		r := ok()
		if checks == 1 {
			r.Disk = 95
		}
		return r
	}
	sent := make(chan string, 4)
	failures := 2
	send := func(s string) error {
		if failures > 0 {
			failures--
			return errors.New("down")
		}
		sent <- s
		return nil
	}
	done := make(chan struct{})
	go func() { Run(ctx, time.Millisecond, limits, check, send); close(done) }()
	select {
	case s := <-sent:
		if !strings.Contains(s, "Disk almost full") {
			t.Errorf("sent %q", s)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("message never sent")
	}
	cancel()
	<-done
}
