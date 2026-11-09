package streaming

/*
#include <stdint.h>
#include <stddef.h>

void *stream_ring_new(size_t capacity);
void stream_ring_free(void *ring);
size_t stream_ring_capacity(const void *ring);
size_t stream_ring_len(const void *ring);
void stream_ring_set_capacity(void *ring, size_t n);
void stream_ring_remember_rgba(void *ring, const uint8_t *data, size_t len, size_t stride, size_t width, size_t height);
void stream_ring_remember_ycbcr(void *ring, const uint8_t *y, size_t ylen, const uint8_t *cb, const uint8_t *cr, size_t clen, size_t ystride, size_t cstride, uint32_t xs, uint32_t ys, size_t width, size_t height);
const uint8_t *stream_ring_frame(const void *ring, size_t i, size_t *width, size_t *height);
*/
import "C"

import (
	"context"
	"image"
	"image/draw"
	"runtime"
	"sync"
	"time"
	"unsafe"

	"github.com/pion/mediadevices"

	_ "security_camera/lib/native"
)

// Handler receives the current frame, the previous frames (oldest first,
// not including the current one, at most Options.History of them) and the
// capture time of the current frame. All images are only valid until the
// handler returns: their buffers are reused for later frames, so copy them
// (e.g. draw them into a new image.RGBA) if you need to keep them.
type Handler func(frame image.Image, history []image.Image, timestamp time.Time)

type Options struct {
	// History is how many past frames are passed to handlers. 0 disables it.
	History int
}

// Stream reads frames from a video track and fans them out to handlers.
type Stream struct {
	track *mediadevices.VideoTrack
	opts  Options

	mu       sync.RWMutex
	handlers []Handler

	// ring buffer of the last History frames, kept in Rust (see
	// rust-lib/streaming/). The frames handlers get point into it, so Run
	// holds ringMu for reading while they run, and anything that changes the
	// ring holds it for writing.
	ringMu sync.RWMutex
	ring   *rustRing
}

// rustRing owns the ring in Rust, freed when it is no longer used.
type rustRing struct{ p unsafe.Pointer }

func newRing(n int) *rustRing {
	r := &rustRing{p: C.stream_ring_new(C.size_t(n))}
	runtime.SetFinalizer(r, func(r *rustRing) { C.stream_ring_free(r.p) })
	return r
}

func New(track *mediadevices.VideoTrack, opts Options) *Stream {
	if opts.History < 0 {
		opts.History = 0
	}
	return &Stream{track: track, opts: opts, ring: newRing(opts.History)}
}

// AddHandler registers a handler. It is safe to call while the stream is running.
func (s *Stream) AddHandler(h Handler) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.handlers = append(s.handlers, h)
}

// Run reads frames until ctx is cancelled or the track fails, calling every
// handler in registration order for each frame. Handlers run synchronously,
// so a slow handler delays the next frame; hand work off to a goroutine if needed.
// Handlers must not modify the images or retain them after returning, and must
// not call SetHistory (it waits for the handlers to return).
func (s *Stream) Run(ctx context.Context) error {
	reader := s.track.NewReader(false)

	for ctx.Err() == nil {
		img, release, err := reader.Read()
		if err != nil {
			return err
		}
		ts := time.Now()

		s.mu.RLock()
		handlers := s.handlers
		s.mu.RUnlock()

		s.ringMu.RLock()
		history := s.history()
		for _, h := range handlers {
			h(img, history, ts)
		}
		s.ringMu.RUnlock()
		s.ringMu.Lock()
		s.remember(img)
		s.ringMu.Unlock()
		release()
	}
	return ctx.Err()
}

// History returns how many past frames are kept.
func (s *Stream) History() int {
	s.ringMu.RLock()
	defer s.ringMu.RUnlock()
	return int(C.stream_ring_capacity(s.ring.p))
}

// SetHistory changes how many past frames are kept. When shrinking, the
// newest frames are kept. It waits for the handlers of the current frame.
func (s *Stream) SetHistory(n int) {
	s.ringMu.Lock()
	defer s.ringMu.Unlock()
	C.stream_ring_set_capacity(s.ring.p, C.size_t(max(n, 0)))
}

// history returns the stored frames, oldest first. They are views of the
// memory of the ring: valid while ringMu is held.
func (s *Stream) history() []image.Image {
	n := int(C.stream_ring_len(s.ring.p))
	out := make([]image.Image, 0, n)
	for i := range n {
		var w, h C.size_t
		p := C.stream_ring_frame(s.ring.p, C.size_t(i), &w, &h)
		out = append(out, &image.RGBA{
			Pix:    unsafe.Slice((*uint8)(unsafe.Pointer(p)), int(w*h*4)),
			Stride: int(w) * 4,
			Rect:   image.Rect(0, 0, int(w), int(h)),
		})
	}
	return out
}

// remember copies img into the ring buffer, forgetting the oldest frame if
// it is full. Frames that are neither RGBA nor YCbCr are converted first.
func (s *Stream) remember(img image.Image) {
	if C.stream_ring_capacity(s.ring.p) == 0 {
		return
	}
	b := img.Bounds()
	if b.Empty() {
		return
	}
	w, h := C.size_t(b.Dx()), C.size_t(b.Dy())
	switch src := img.(type) {
	case *image.RGBA:
		pix := src.Pix[src.PixOffset(b.Min.X, b.Min.Y):]
		C.stream_ring_remember_rgba(s.ring.p, (*C.uint8_t)(unsafe.Pointer(&pix[0])), C.size_t(len(pix)), C.size_t(src.Stride), w, h)
	case *image.YCbCr:
		var xs, ys C.uint32_t
		switch src.SubsampleRatio {
		case image.YCbCrSubsampleRatio422:
			xs = 1
		case image.YCbCrSubsampleRatio420:
			xs, ys = 1, 1
		case image.YCbCrSubsampleRatio440:
			ys = 1
		case image.YCbCrSubsampleRatio411:
			xs = 2
		case image.YCbCrSubsampleRatio410:
			xs, ys = 2, 1
		}
		y := src.Y[src.YOffset(b.Min.X, b.Min.Y):]
		co := src.COffset(b.Min.X, b.Min.Y)
		cb, cr := src.Cb[co:], src.Cr[co:]
		C.stream_ring_remember_ycbcr(s.ring.p, (*C.uint8_t)(unsafe.Pointer(&y[0])), C.size_t(len(y)),
			(*C.uint8_t)(unsafe.Pointer(&cb[0])), (*C.uint8_t)(unsafe.Pointer(&cr[0])), C.size_t(len(cb)),
			C.size_t(src.YStride), C.size_t(src.CStride), xs, ys, w, h)
	default:
		rgba := image.NewRGBA(image.Rect(0, 0, b.Dx(), b.Dy()))
		draw.Draw(rgba, rgba.Bounds(), img, b.Min, draw.Src)
		s.remember(rgba)
	}
}
