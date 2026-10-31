// Package alarmqueue keeps track of the alarms and sends their Telegram
// messages, never more than a set number per minute.
//
// An alarm starts as Processing (the GIFs are being made), becomes Queued
// when its message is ready and waits for its turn, and ends as Sent, Failed
// or Cancelled.
package alarmqueue

import (
	"context"
	"fmt"
	"os"
	"strings"
	"sync"
	"time"
)

// Status is where an alarm is in its life.
type Status int

const (
	// Processing: the message is still being made.
	Processing Status = iota
	// Queued: the message is ready and waits for the rate limit.
	Queued
	Sent
	// Failed: making or sending the message failed.
	Failed
	// Cancelled: the alarm was dropped before it was sent.
	Cancelled
)

const (
	// kept is how many alarms are remembered.
	kept = 100
	// maxQueued is how many messages can wait to be sent; past that the
	// oldest is dropped, so the memory of the GIFs they hold stays bounded.
	maxQueued = 20
)

// Queue sends the messages of the alarms. The zero value is not usable: use New.
type Queue struct {
	perMinute int
	window    time.Duration
	now       func() time.Time

	mu     sync.Mutex
	alarms []*Alarm // oldest first, at most kept
	ready  []*Alarm // Queued alarms in the order they will be sent
	sent   []time.Time
	wake   chan struct{}
}

// New returns a Queue that sends at most perMinute messages per minute.
// Start it with Run.
func New(perMinute int) *Queue {
	return newQueue(perMinute, time.Minute)
}

func newQueue(perMinute int, window time.Duration) *Queue {
	return &Queue{perMinute: max(perMinute, 1), window: window, now: time.Now, wake: make(chan struct{}, 1)}
}

// Alarm is one alarm in a Queue.
type Alarm struct {
	q        *Queue
	at       time.Time
	detector string
	score    float64

	// guarded by q.mu
	status Status
	sentAt time.Time
	reason string
	send   func() error
}

// Begin records a new alarm detected at at, in the Processing state.
func (q *Queue) Begin(at time.Time, detector string, score float64) *Alarm {
	a := &Alarm{q: q, at: at, detector: detector, score: score}
	q.mu.Lock()
	defer q.mu.Unlock()
	q.alarms = append(q.alarms, a)
	if len(q.alarms) > kept {
		q.alarms = q.alarms[len(q.alarms)-kept:]
	}
	return a
}

// Ready says the message of the alarm is made: send sends it, when the rate
// limit allows. It does nothing if the alarm is no longer Processing.
func (a *Alarm) Ready(send func() error) {
	q := a.q
	q.mu.Lock()
	defer q.mu.Unlock()
	if a.status != Processing {
		return
	}
	a.status, a.send = Queued, send
	q.ready = append(q.ready, a)
	if len(q.ready) > maxQueued {
		old := q.ready[0]
		q.ready = q.ready[1:]
		old.status, old.reason, old.send = Cancelled, "queue full", nil
	}
	select {
	case q.wake <- struct{}{}:
	default:
	}
}

// Fail ends the alarm because its message could not be made.
func (a *Alarm) Fail(err error) { a.end(Failed, err.Error()) }

// Cancel ends the alarm because it is not to be sent, e.g. "paused".
func (a *Alarm) Cancel(reason string) { a.end(Cancelled, reason) }

func (a *Alarm) end(s Status, reason string) {
	a.q.mu.Lock()
	defer a.q.mu.Unlock()
	if a.status == Processing {
		a.status, a.reason = s, reason
	}
}

// Run sends the queued messages until ctx is done.
func (q *Queue) Run(ctx context.Context) {
	for {
		a := q.next(ctx)
		if a == nil {
			return
		}
		err := a.send()
		q.mu.Lock()
		if err != nil {
			a.status, a.reason = Failed, err.Error()
			fmt.Fprintln(os.Stderr, "alarm:", err)
		} else {
			a.status, a.sentAt = Sent, q.now()
		}
		a.send = nil
		q.mu.Unlock()
	}
}

// next waits for a queued message and for the rate limit to allow it. It
// returns nil when ctx is done.
func (q *Queue) next(ctx context.Context) *Alarm {
	for {
		q.mu.Lock()
		wait := time.Duration(-1)
		if len(q.ready) > 0 {
			now := q.now()
			for len(q.sent) > 0 && now.Sub(q.sent[0]) >= q.window {
				q.sent = q.sent[1:]
			}
			if len(q.sent) < q.perMinute {
				a := q.ready[0]
				q.ready = q.ready[1:]
				q.sent = append(q.sent, now)
				q.mu.Unlock()
				return a
			}
			wait = q.sent[0].Add(q.window).Sub(now)
		}
		q.mu.Unlock()

		var timeout <-chan time.Time
		var timer *time.Timer
		if wait >= 0 {
			timer = time.NewTimer(wait)
			timeout = timer.C
		}
		select {
		case <-ctx.Done():
			if timer != nil {
				timer.Stop()
			}
			return nil
		case <-q.wake:
		case <-timeout:
		}
		if timer != nil {
			timer.Stop()
		}
	}
}

// Info is the state of an alarm.
type Info struct {
	At       time.Time
	Detector string
	Score    float64
	Status   Status
	// Position is the place of a Queued alarm in the queue, 1 for the next to be sent.
	Position int
	SentAt   time.Time
	// Reason is why a Failed or Cancelled alarm was not sent.
	Reason string
}

// Recent returns the state of the latest n alarms, newest first.
func (q *Queue) Recent(n int) []Info {
	q.mu.Lock()
	defer q.mu.Unlock()
	var out []Info
	for i := len(q.alarms) - 1; i >= 0 && len(out) < n; i-- {
		a := q.alarms[i]
		info := Info{At: a.at, Detector: a.detector, Score: a.score, Status: a.status, SentAt: a.sentAt, Reason: a.reason}
		for j, r := range q.ready {
			if r == a {
				info.Position = j + 1
			}
		}
		out = append(out, info)
	}
	return out
}

// Limit returns the most messages sent per minute.
func (q *Queue) Limit() int { return q.perMinute }

// Report describes the latest n alarms for a Telegram message.
func (q *Queue) Report(n int) string {
	alarms := q.Recent(n)
	if len(alarms) == 0 {
		return fmt.Sprintf("No alarms yet (at most %d alarm messages are sent per minute).", q.perMinute)
	}
	var b strings.Builder
	fmt.Fprintf(&b, "Latest %d alarms, newest first (at most %d messages per minute):", len(alarms), q.perMinute)
	for _, a := range alarms {
		fmt.Fprintf(&b, "\n%s %s %.4f: ", a.At.Format("15:04:05"), a.Detector, a.Score)
		switch a.Status {
		case Processing:
			b.WriteString("being processed")
		case Queued:
			fmt.Fprintf(&b, "waiting to be sent (%d in the queue)", a.Position)
		case Sent:
			fmt.Fprintf(&b, "sent at %s", a.SentAt.Format("15:04:05"))
		case Failed:
			fmt.Fprintf(&b, "failed (%s)", a.Reason)
		case Cancelled:
			fmt.Fprintf(&b, "not sent (%s)", a.Reason)
		}
	}
	return b.String()
}
