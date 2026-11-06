package alarmlog

import (
	"errors"
	"strings"
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
	if len(l.byMessage) != kept {
		t.Errorf("%d messages remembered, want %d", len(l.byMessage), kept)
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
