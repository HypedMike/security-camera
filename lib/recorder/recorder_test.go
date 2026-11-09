package recorder

import (
	"image"
	"testing"
	"time"
)

func TestRecordsOnTheGridThenFinishes(t *testing.T) {
	r := New(func(img image.Image) image.Image { return img })
	img := image.NewRGBA(image.Rect(0, 0, 1, 1))
	done := make(chan []image.Image, 1)
	t0 := time.Unix(1000, 0)

	r.Start(t0, img, 5*time.Second, 500*time.Millisecond, func(f []image.Image) { done <- f })
	// a 30 fps camera
	for i := 1; i <= 160; i++ {
		r.Frame(img, t0.Add(time.Duration(i)*time.Second/30))
	}
	select {
	case frames := <-done:
		if len(frames) != 10 {
			t.Errorf("%d frames, want 10 (2 a second for 5 seconds)", len(frames))
		}
	case <-time.After(time.Second):
		t.Fatal("the recording never finished")
	}
	if r.active() != 0 {
		t.Errorf("%d recordings still active", r.active())
	}
}

func TestSlowCameraStillFinishes(t *testing.T) {
	r := New(func(img image.Image) image.Image { return img })
	img := image.NewRGBA(image.Rect(0, 0, 1, 1))
	done := make(chan []image.Image, 1)
	t0 := time.Unix(1000, 0)
	r.Start(t0, img, 5*time.Second, 0, func(f []image.Image) { done <- f })
	r.Frame(img, t0.Add(10*time.Second))
	if f := <-done; len(f) != 1 {
		t.Errorf("%d frames, want just the first", len(f))
	}
}

func TestOverlappingRecordings(t *testing.T) {
	r := New(func(img image.Image) image.Image { return img })
	img := image.NewRGBA(image.Rect(0, 0, 1, 1))
	a, b := make(chan []image.Image, 1), make(chan []image.Image, 1)
	t0 := time.Unix(1000, 0)
	r.Start(t0, img, 2*time.Second, time.Second, func(f []image.Image) { a <- f })
	r.Start(t0.Add(time.Second), img, 2*time.Second, time.Second, func(f []image.Image) { b <- f })
	for i := 1; i <= 40; i++ {
		r.Frame(img, t0.Add(time.Duration(i)*100*time.Millisecond))
	}
	if f := <-a; len(f) != 2 {
		t.Errorf("first: %d frames, want 2", len(f))
	}
	if f := <-b; len(f) != 2 {
		t.Errorf("second: %d frames, want 2", len(f))
	}
}
