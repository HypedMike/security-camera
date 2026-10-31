package alarmqueue

import (
	"context"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"
)

func start(t *testing.T, q *Queue) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() { q.Run(ctx); close(done) }()
	t.Cleanup(func() { cancel(); <-done })
}

func waitFor(t *testing.T, what string, cond func() bool) {
	t.Helper()
	for i := 0; i < 400; i++ {
		if cond() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", what)
}

func TestSendsAtMostPerWindow(t *testing.T) {
	const window = 200 * time.Millisecond
	q := newQueue(2, window)
	start(t, q)

	var mu sync.Mutex
	var times []time.Time
	send := func() error {
		mu.Lock()
		defer mu.Unlock()
		times = append(times, time.Now())
		return nil
	}
	begin := time.Now()
	for i := 0; i < 5; i++ {
		q.Begin(begin, "background", 0.01).Ready(send)
	}
	waitFor(t, "5 sends", func() bool { mu.Lock(); defer mu.Unlock(); return len(times) == 5 })

	// no window holds more than 2 sends
	for i := 2; i < len(times); i++ {
		if d := times[i].Sub(times[i-2]); d < window-20*time.Millisecond {
			t.Fatalf("sends %d and %d are %v apart, less than the %v window", i-2, i, d, window)
		}
	}
	if d := times[1].Sub(begin); d > window/2 {
		t.Fatalf("the first two should go at once, the second took %v", d)
	}
}

func TestStatuses(t *testing.T) {
	q := newQueue(1, time.Hour)
	at := time.Date(2026, 1, 2, 3, 4, 5, 0, time.UTC)

	processing := q.Begin(at, "background", 0.5)
	failed := q.Begin(at, "background", 0.5)
	cancelled := q.Begin(at, "background", 0.5)
	failed.Fail(errors.New("boom"))
	cancelled.Cancel("paused")
	// Cancel and Fail after the end change nothing
	cancelled.Fail(errors.New("later"))

	got := q.Recent(10)
	if len(got) != 3 {
		t.Fatalf("got %d alarms", len(got))
	}
	// newest first
	if got[0].Status != Cancelled || got[0].Reason != "paused" {
		t.Errorf("cancelled: %+v", got[0])
	}
	if got[1].Status != Failed || got[1].Reason != "boom" {
		t.Errorf("failed: %+v", got[1])
	}
	if got[2].Status != Processing {
		t.Errorf("processing: %+v", got[2])
	}
	_ = processing
}

func TestQueuedPositionsAndSent(t *testing.T) {
	q := newQueue(1, time.Hour)
	start(t, q)
	at := time.Date(2026, 1, 2, 3, 4, 5, 0, time.UTC)

	first := q.Begin(at, "background", 0.1)
	second := q.Begin(at, "background", 0.2)
	third := q.Begin(at, "background", 0.3)
	first.Ready(func() error { return nil })
	waitFor(t, "first sent", func() bool { return q.Recent(3)[2].Status == Sent })
	second.Ready(func() error { return nil })
	third.Ready(func() error { return nil })

	// the limit is 1 per hour and one went out: the others wait in order
	got := q.Recent(3)
	if got[1].Status != Queued || got[1].Position != 1 || got[0].Status != Queued || got[0].Position != 2 {
		t.Fatalf("queue: %+v", got)
	}
	if got[2].SentAt.IsZero() {
		t.Fatal("sent alarm has no sent time")
	}
}

func TestFailedSend(t *testing.T) {
	q := newQueue(5, time.Minute)
	start(t, q)
	a := q.Begin(time.Now(), "background", 0.1)
	a.Ready(func() error { return errors.New("telegram down") })
	waitFor(t, "failure", func() bool { return q.Recent(1)[0].Status == Failed })
	if r := q.Recent(1)[0].Reason; r != "telegram down" {
		t.Fatalf("reason %q", r)
	}
}

func TestQueueFullDropsTheOldest(t *testing.T) {
	q := newQueue(1, time.Hour) // not running: nothing is sent
	var first *Alarm
	for i := 0; i < maxQueued+1; i++ {
		a := q.Begin(time.Now(), "background", 0.1)
		if i == 0 {
			first = a
		}
		a.Ready(func() error { return nil })
	}
	all := q.Recent(maxQueued + 1)
	oldest := all[len(all)-1]
	if oldest.Status != Cancelled || oldest.Reason != "queue full" {
		t.Fatalf("oldest: %+v", oldest)
	}
	if first.send != nil {
		t.Fatal("dropped alarm still holds its message")
	}
	if len(q.ready) != maxQueued {
		t.Fatalf("%d queued, want %d", len(q.ready), maxQueued)
	}
}

func TestKeepsOnlyRecentAlarms(t *testing.T) {
	q := newQueue(1, time.Minute)
	for i := 0; i < kept+10; i++ {
		q.Begin(time.Now(), "background", 0.1)
	}
	if n := len(q.Recent(1000)); n != kept {
		t.Fatalf("kept %d alarms, want %d", n, kept)
	}
}

func TestReport(t *testing.T) {
	q := newQueue(2, time.Minute)
	if got := q.Report(10); !strings.Contains(got, "No alarms yet") {
		t.Fatalf("empty report: %q", got)
	}
	at := time.Date(2026, 1, 2, 14, 3, 22, 0, time.UTC)
	q.Begin(at, "background", 0.0041)
	got := q.Report(10)
	for _, want := range []string{"at most 2 messages per minute", "14:03:22 background 0.0041: being processed"} {
		if !strings.Contains(got, want) {
			t.Errorf("report lacks %q:\n%s", want, got)
		}
	}
}
