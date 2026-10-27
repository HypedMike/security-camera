package streaming

import (
	"image"
	"image/color"
	"testing"
)

func TestHistoryOrder(t *testing.T) {
	s := &Stream{ring: make([]*image.RGBA, 3)}
	for i := 1; i <= 5; i++ {
		img := image.NewRGBA(image.Rect(0, 0, 1, 1))
		img.Set(0, 0, color.RGBA{R: uint8(i), A: 255})
		h := s.history()
		if want := min(i-1, 3); len(h) != want {
			t.Fatalf("frame %d: len=%d want %d", i, len(h), want)
		}
		for j, f := range h {
			r, _, _, _ := f.At(0, 0).RGBA()
			if want := uint32(i-len(h)+j) * 0x101; r != want {
				t.Fatalf("frame %d idx %d: got %x want %x", i, j, r, want)
			}
		}
		s.remember(img)
	}
}
