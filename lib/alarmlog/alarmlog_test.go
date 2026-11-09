package alarmlog

import (
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestRequestAfterReady(t *testing.T) {
	var l Log
	a := l.Begin(time.Now(), "background", 0.1, "abc123")
	a.Posted(7)
	if _, act := l.Request(7); act != Later {
		t.Fatalf("while recording: %v, want Later", act)
	}
	if !a.Finish(nil) {
		t.Fatal("somebody asked before it was ready: Finish should say to send")
	}
	if _, act := l.Request(7); act != Nothing {
		t.Errorf("after it is being sent: %v, want Nothing", act)
	}
	a.Unsent()
	if got, act := l.Request(7); act != Send || got != a {
		t.Errorf("after a failed send: %v, want Send", act)
	}
}

func TestRequestWhenAlreadyReady(t *testing.T) {
	var l Log
	a := l.Begin(time.Now(), "background", 0.1, "abc123")
	a.Posted(7)
	if a.Finish(nil) {
		t.Fatal("nobody asked: Finish must not send")
	}
	if _, act := l.Request(7); act != Send {
		t.Errorf("got %v, want Send", act)
	}
	if _, act := l.Request(7); act != Nothing {
		t.Errorf("second thumbs up: %v, want Nothing", act)
	}
}

func TestFailedAndUnknown(t *testing.T) {
	var l Log
	a := l.Begin(time.Now(), "background", 0.1, "abc123")
	a.Posted(7)
	if a.Finish(errors.New("boom")) {
		t.Error("a failed GIF must not be sent")
	}
	if _, act := l.Request(7); act != NoGIF {
		t.Errorf("got %v, want NoGIF", act)
	}
	if _, act := l.Request(99); act != Unknown {
		t.Errorf("got %v, want Unknown", act)
	}
}

func TestOldAlarmsForgotten(t *testing.T) {
	var l Log
	for i := 1; i <= kept+5; i++ {
		l.Begin(time.Now(), "d", 0, "c").Posted(i)
	}
	if _, act := l.Request(1); act != Unknown {
		t.Errorf("alarm 1 should be forgotten, got %v", act)
	}
	if _, act := l.Request(kept + 5); act == Unknown {
		t.Error("the latest alarm should be remembered")
	}
	// the oldest one remembered is the one kept alarms before the latest
	if _, act := l.Request(5); act != Unknown {
		t.Errorf("alarm 5 should be forgotten, got %v", act)
	}
	if _, act := l.Request(6); act == Unknown {
		t.Error("alarm 6 should still be remembered")
	}
	if len(l.alarms) != kept {
		t.Errorf("%d alarms remembered, want %d", len(l.alarms), kept)
	}
}

func TestEmbeddedAndConcurrent(t *testing.T) {
	// a Log can be part of a bigger struct, and be used from many goroutines
	var s struct {
		pad int
		Log
	}
	var wg sync.WaitGroup
	for i := 1; i <= 50; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			a := s.Begin(time.Now(), "d", 0.5, "code")
			a.Posted(i)
			s.Request(i)
			a.Finish(nil)
			_ = s.Report(3)
		}()
	}
	wg.Wait()
	if got := len(s.alarms); got != 50 {
		t.Errorf("%d alarms, want 50", got)
	}
}

func TestReportFormat(t *testing.T) {
	var l Log
	at := time.Date(2026, 11, 8, 23, 4, 5, 0, time.FixedZone("x", 3600))
	l.Begin(at, "background", 0.03125, "").Finish(errors.New("disk full"))
	want := "Latest 1 alarms, newest first:\n23:04:05 background 0.0312: no GIF (disk full)"
	if got := l.Report(5); got != want {
		t.Errorf("got %q, want %q", got, want)
	}
	// same as Go's own time and number formatting
	if got, w := fmt.Sprintf("%s %.4f", at.Format("15:04:05"), 0.03125), "23:04:05 0.0312"; got != w {
		t.Fatalf("Go prints %q, want %q", got, w)
	}
	if got := l.Report(0); got != "No alarms yet." {
		t.Errorf("Report(0): %q", got)
	}
}

func TestReport(t *testing.T) {
	var l Log
	if got := l.Report(5); got != "No alarms yet." {
		t.Errorf("got %q", got)
	}
	l.Begin(time.Now(), "background", 0.1, "aaaaaa")
	b := l.Begin(time.Now(), "background", 0.2, "bbbbbb")
	b.Finish(nil)
	got := l.Report(5)
	if !strings.Contains(got, "GIF ready, react with 👍 or /hd bbbbbb") || !strings.Contains(got, "recording the GIF") {
		t.Errorf("report: %s", got)
	}
}
