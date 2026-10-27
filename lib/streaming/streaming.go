package streaming

import (
	"context"
	"image"
	"image/draw"
	"sync"
	"time"

	"github.com/pion/mediadevices"
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

	// ring buffer of the last opts.History frames, owned by Run
	ring  []*image.RGBA
	next  int
	count int
}

func New(track *mediadevices.VideoTrack, opts Options) *Stream {
	if opts.History < 0 {
		opts.History = 0
	}
	return &Stream{track: track, opts: opts, ring: make([]*image.RGBA, opts.History)}
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
// Handlers must not modify the images or retain them after returning.
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

		history := s.history()
		for _, h := range handlers {
			h(img, history, ts)
		}
		s.remember(img)
		release()
	}
	return ctx.Err()
}

// history returns the stored frames, oldest first.
func (s *Stream) history() []image.Image {
	out := make([]image.Image, 0, s.count)
	for i := 0; i < s.count; i++ {
		out = append(out, s.ring[(s.next-s.count+i+len(s.ring))%len(s.ring)])
	}
	return out
}

// remember copies img into the ring buffer, reusing the slot's memory when possible.
func (s *Stream) remember(img image.Image) {
	if len(s.ring) == 0 {
		return
	}
	b := img.Bounds()
	dst := s.ring[s.next]
	if dst == nil || dst.Bounds() != b {
		dst = image.NewRGBA(b)
		s.ring[s.next] = dst
	}
	draw.Draw(dst, b, img, b.Min, draw.Src)
	s.next = (s.next + 1) % len(s.ring)
	if s.count < len(s.ring) {
		s.count++
	}
}
