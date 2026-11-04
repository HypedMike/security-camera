// Package hdgif makes the HD alarm GIFs in the background, one at a time.
//
// An HD GIF is slow to encode (tens of seconds for 75 dithered 640x480
// frames, more on a small board) and its frames take about 90 MB until it is
// done. The alarm message must not wait for it, and during a long motion
// the GIFs must not all be made at once: they would share the CPU, each would
// get slower, and their frames would fill the memory. So the jobs wait in a
// short queue and a single worker makes them; past the limit the oldest
// waiting job is dropped.
package hdgif

import (
	"context"
	"fmt"
	"os"
	"sync"
)

// State is where the HD GIF of a code is.
type State int

const (
	// Unknown: no job with this code, it is either saved or was never made.
	Unknown State = iota
	// Pending: the GIF is waiting for its turn or being made.
	Pending
	// Dropped: the GIF was not made, the queue was full or it failed.
	Dropped
)

// keptDropped is how many dropped codes are remembered to answer /hd.
const keptDropped = 50

// Queue makes the HD GIFs. The zero value is not usable: use New.
type Queue struct {
	max int

	mu      sync.Mutex
	jobs    []job // waiting, oldest first
	busy    string
	dropped []string // oldest first, at most keptDropped
	wake    chan struct{}
}

type job struct {
	code string
	// make encodes the GIF and saves it under code; it holds the frames
	make func() error
}

// New returns a Queue where at most max jobs wait, besides the one being made.
// Start it with Run.
func New(max int) *Queue {
	return &Queue{max: max, wake: make(chan struct{}, 1)}
}

// Add queues the HD GIF of code: mk encodes and saves it. If the queue is full
// the oldest waiting job is dropped and its code returned, otherwise "".
func (q *Queue) Add(code string, mk func() error) (dropped string) {
	q.mu.Lock()
	q.jobs = append(q.jobs, job{code, mk})
	if len(q.jobs) > q.max {
		dropped = q.jobs[0].code
		q.jobs[0] = job{} // let the frames be freed
		q.jobs = q.jobs[1:]
		q.drop(dropped)
	}
	q.mu.Unlock()
	select {
	case q.wake <- struct{}{}:
	default:
	}
	return dropped
}

// drop remembers that the GIF of code will not be made. q.mu must be held.
func (q *Queue) drop(code string) {
	q.dropped = append(q.dropped, code)
	if len(q.dropped) > keptDropped {
		q.dropped = q.dropped[len(q.dropped)-keptDropped:]
	}
}

// State returns where the HD GIF of code is.
func (q *Queue) State(code string) State {
	q.mu.Lock()
	defer q.mu.Unlock()
	if q.busy == code {
		return Pending
	}
	for _, j := range q.jobs {
		if j.code == code {
			return Pending
		}
	}
	for _, c := range q.dropped {
		if c == code {
			return Dropped
		}
	}
	return Unknown
}

// Run makes the queued GIFs, one at a time, until ctx is done.
func (q *Queue) Run(ctx context.Context) {
	for {
		q.mu.Lock()
		if len(q.jobs) == 0 {
			q.mu.Unlock()
			select {
			case <-ctx.Done():
				return
			case <-q.wake:
			}
			continue
		}
		j := q.jobs[0]
		q.jobs[0] = job{}
		q.jobs = q.jobs[1:]
		q.busy = j.code
		q.mu.Unlock()

		err := j.make()

		q.mu.Lock()
		q.busy = ""
		if err != nil {
			fmt.Fprintln(os.Stderr, "hd gif:", err)
			q.drop(j.code)
		}
		q.mu.Unlock()
		if ctx.Err() != nil {
			return
		}
	}
}
