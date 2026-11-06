// Package recorder captures a few frames of the camera over a span of time,
// starting when an alarm goes off, without holding up the stream.
package recorder

import (
	"image"
	"sync"
	"time"
)

// Recorder records several clips at once, one per Start. The zero value is not
// usable: use New.
type Recorder struct {
	mu   sync.Mutex
	recs []*recording
	// shrink copies a frame so it stays valid after the stream reuses its buffer.
	shrink func(image.Image) image.Image
}

type recording struct {
	end, next time.Time
	interval  time.Duration
	frames    []image.Image
	done      func([]image.Image)
}

// New returns a Recorder. copyFrame returns a copy of a frame that stays valid
// after the camera's buffer is reused, possibly scaled down.
func New(copyFrame func(image.Image) image.Image) *Recorder {
	return &Recorder{shrink: copyFrame}
}

// Start begins a clip with frame, taken at at, then one frame every interval
// until span has passed. done is then called, in its own goroutine, with the
// frames. Frame feeds the following frames.
func (r *Recorder) Start(at time.Time, frame image.Image, span, interval time.Duration, done func([]image.Image)) {
	if interval <= 0 {
		interval = span
	}
	rec := &recording{end: at.Add(span), next: at.Add(interval), interval: interval, done: done}
	rec.frames = append(rec.frames, r.shrink(frame))
	r.mu.Lock()
	defer r.mu.Unlock()
	r.recs = append(r.recs, rec)
}

// Frame is called for every frame of the stream: it keeps the ones the clips
// being recorded are waiting for, and finishes the clips whose span is over.
func (r *Recorder) Frame(frame image.Image, ts time.Time) {
	r.mu.Lock()
	defer r.mu.Unlock()
	live := r.recs[:0]
	for _, rec := range r.recs {
		if !ts.Before(rec.end) {
			go rec.done(rec.frames)
			continue
		}
		if !ts.Before(rec.next) {
			rec.frames = append(rec.frames, r.shrink(frame))
			// stay on the grid even if a frame came late
			for !ts.Before(rec.next) {
				rec.next = rec.next.Add(rec.interval)
			}
		}
		live = append(live, rec)
	}
	clear(r.recs[len(live):])
	r.recs = live
}
