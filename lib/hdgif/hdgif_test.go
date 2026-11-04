package hdgif

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
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

func TestMakesOneAtATimeInOrder(t *testing.T) {
	q := New(10)
	start(t, q)

	var running, most atomic.Int32
	var mu sync.Mutex
	var order []string
	job := func(code string) func() error {
		return func() error {
			n := running.Add(1)
			defer running.Add(-1)
			if n > most.Load() {
				most.Store(n)
			}
			time.Sleep(10 * time.Millisecond)
			mu.Lock()
			order = append(order, code)
			mu.Unlock()
			return nil
		}
	}
	for _, c := range []string{"aaaaaa", "bbbbbb", "cccccc"} {
		q.Add(c, job(c))
	}
	waitFor(t, "all made", func() bool { mu.Lock(); defer mu.Unlock(); return len(order) == 3 })
	if most.Load() != 1 {
		t.Errorf("%d GIFs made at once, want 1", most.Load())
	}
	if order[0] != "aaaaaa" || order[1] != "bbbbbb" || order[2] != "cccccc" {
		t.Errorf("order %v", order)
	}
	if s := q.State("aaaaaa"); s != Unknown {
		t.Errorf("made GIF state %v, want Unknown", s)
	}
}

func TestFullQueueDropsTheOldestWaiting(t *testing.T) {
	q := New(2)
	start(t, q)

	release := make(chan struct{})
	var made sync.Map
	job := func(code string) func() error {
		return func() error { <-release; made.Store(code, true); return nil }
	}
	q.Add("000001", job("000001"))
	waitFor(t, "first busy", func() bool { q.mu.Lock(); defer q.mu.Unlock(); return q.busy == "000001" })

	q.Add("000002", job("000002"))
	q.Add("000003", job("000003"))
	if d := q.Add("000004", job("000004")); d != "000002" {
		t.Fatalf("dropped %q, want 000002", d)
	}
	for code, want := range map[string]State{"000001": Pending, "000002": Dropped, "000003": Pending, "000004": Pending, "ffffff": Unknown} {
		if s := q.State(code); s != want {
			t.Errorf("state of %s = %v, want %v", code, s, want)
		}
	}
	close(release)
	waitFor(t, "rest made", func() bool { _, ok := made.Load("000004"); return ok })
	if _, ok := made.Load("000002"); ok {
		t.Error("dropped GIF was made")
	}
}

func TestFailedIsDropped(t *testing.T) {
	q := New(2)
	start(t, q)
	q.Add("abcdef", func() error { return errors.New("disk full") })
	waitFor(t, "failure", func() bool { return q.State("abcdef") == Dropped })
}
