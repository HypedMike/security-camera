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

func TestSetHistoryKeepsNewest(t *testing.T) {
	s := &Stream{ring: make([]*image.RGBA, 4)}
	add := func(i int) {
		img := image.NewRGBA(image.Rect(0, 0, 1, 1))
		img.Set(0, 0, color.RGBA{R: uint8(i), A: 255})
		s.remember(img)
	}
	check := func(want ...int) {
		t.Helper()
		h := s.history()
		if len(h) != len(want) {
			t.Fatalf("len=%d want %d", len(h), len(want))
		}
		for j, f := range h {
			if r, _, _, _ := f.At(0, 0).RGBA(); r != uint32(want[j])*0x101 {
				t.Fatalf("idx %d: got %x want %d", j, r, want[j])
			}
		}
	}
	for i := 1; i <= 6; i++ {
		add(i)
	}
	check(3, 4, 5, 6)
	s.SetHistory(2)
	check(5, 6)
	add(7)
	check(6, 7)
	s.SetHistory(5)
	check(6, 7)
	add(8)
	check(6, 7, 8)
}
