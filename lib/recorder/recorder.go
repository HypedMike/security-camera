// Package recorder captures a few frames of the camera over a span of time,
// starting when an alarm goes off, without holding up the stream.
//
// The timing (which frame belongs to which clip) is decided in Rust, see
// rust-lib/recorder/. The frames stay here, as Go images.
package recorder

/*
#include <stdint.h>
#include <stddef.h>

void *recorder_new(void);
void recorder_free(void *rec);
uint64_t recorder_start(void *rec, int64_t at, int64_t span, int64_t interval);
size_t recorder_frame(void *rec, int64_t ts);
int32_t recorder_event(const void *rec, size_t i, uint64_t *clip);
size_t recorder_active(const void *rec);
*/
import "C"

import (
	"image"
	"runtime"
	"sync"
	"time"
	"unsafe"

	_ "security_camera/lib/native"
)

// epoch is where the times given to the Rust side start from. Measuring from
// a time.Now() keeps the monotonic clock in use, so a change of the wall clock
// does not disturb a recording.
var epoch = time.Now()

func nanos(t time.Time) C.int64_t { return C.int64_t(t.Sub(epoch)) }

// Recorder records several clips at once, one per Start. The zero value is not
// usable: use New.
type Recorder struct {
	mu    sync.Mutex
	rec   unsafe.Pointer // the Rust recorder, freed when the Recorder is no longer used
	clips map[uint64]*clip
	// shrink copies a frame so it stays valid after the stream reuses its buffer.
	shrink func(image.Image) image.Image
}

type clip struct {
	frames []image.Image
	done   func([]image.Image)
}

// New returns a Recorder. copyFrame returns a copy of a frame that stays valid
// after the camera's buffer is reused, possibly scaled down.
func New(copyFrame func(image.Image) image.Image) *Recorder {
	r := &Recorder{rec: C.recorder_new(), clips: map[uint64]*clip{}, shrink: copyFrame}
	runtime.SetFinalizer(r, func(r *Recorder) { C.recorder_free(r.rec) })
	return r
}

// Start begins a clip with frame, taken at at, then one frame every interval
// until span has passed. done is then called, in its own goroutine, with the
// frames. Frame feeds the following frames.
func (r *Recorder) Start(at time.Time, frame image.Image, span, interval time.Duration, done func([]image.Image)) {
	first := r.shrink(frame)
	r.mu.Lock()
	defer r.mu.Unlock()
	id := uint64(C.recorder_start(r.rec, nanos(at), C.int64_t(span), C.int64_t(interval)))
	r.clips[id] = &clip{frames: []image.Image{first}, done: done}
}

// Frame is called for every frame of the stream: it keeps the ones the clips
// being recorded are waiting for, and finishes the clips whose span is over.
func (r *Recorder) Frame(frame image.Image, ts time.Time) {
	r.mu.Lock()
	defer r.mu.Unlock()
	n := int(C.recorder_frame(r.rec, nanos(ts)))
	for i := range n {
		var id C.uint64_t
		keep := C.recorder_event(r.rec, C.size_t(i), &id)
		c := r.clips[uint64(id)]
		if c == nil {
			continue
		}
		if keep == 1 {
			c.frames = append(c.frames, r.shrink(frame))
			continue
		}
		delete(r.clips, uint64(id))
		go c.done(c.frames)
	}
}

// active is how many clips are being recorded.
func (r *Recorder) active() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return int(C.recorder_active(r.rec))
}
