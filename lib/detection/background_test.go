package detection

import (
	"image"
	"image/color"
	"math/rand"
	"testing"
)

// The defaults of the Rust detector (rust-lib/detection/src/lib.rs).
const (
	bgSettle = 10
	bgStill  = 60
	bgWindow = 5
)

// scene is a 640x480 gray frame with sensor noise, plus an optional square of
// side size at (x, y) and an overall brightness shift.
func scene(rng *rand.Rand, x, y, size int, shift int) *image.RGBA {
	img := image.NewRGBA(image.Rect(0, 0, 640, 480))
	for py := 0; py < 480; py++ {
		for px := 0; px < 640; px++ {
			v := 100 + px/16 + shift + rng.Intn(9) - 4 // a gentle gradient, ±4 noise
			if size > 0 && px >= x && px < x+size && py >= y && py < y+size {
				v = 30
			}
			img.SetRGBA(px, py, color.RGBA{uint8(v), uint8(v), uint8(v), 255})
		}
	}
	return img
}

// warm feeds still frames until the detector has learned the scene.
func warm(t *testing.T, d *Background, rng *rand.Rand) {
	t.Helper()
	for i := 0; i < 40; i++ {
		if s, err := d.Score(scene(rng, 0, 0, 0, 0), nil); err != nil {
			t.Fatal(err)
		} else if i >= 20 && s != 0 {
			t.Fatalf("still frame %d scored %v", i, s)
		}
	}
}

func TestBackgroundSmallMotion(t *testing.T) {
	rng := rand.New(rand.NewSource(1))
	d := NewBackground()
	warm(t, d, rng)

	// a 16x16 square (0.08% of the frame) moving in a corner
	var s float64
	for i := 0; i < 5; i++ {
		s, _ = d.Score(scene(rng, 560+4*i, 40, 16, 0), nil)
	}
	if s < 0.0005 {
		t.Fatalf("small moving square scored %v", s)
	}
	if s > 0.01 {
		t.Fatalf("small moving square scored %v, larger than the square", s)
	}
}

func TestBackgroundPersistence(t *testing.T) {
	rng := rand.New(rand.NewSource(2))
	d := NewBackground()
	warm(t, d, rng)

	// one frame with an object is not enough
	if s, _ := d.Score(scene(rng, 300, 200, 40, 0), nil); s != 0 {
		t.Fatalf("single frame scored %v", s)
	}
	d.Score(scene(rng, 0, 0, 0, 0), nil)
	if s, _ := d.Score(scene(rng, 0, 0, 0, 0), nil); s != 0 {
		t.Fatalf("glitch then still scored %v", s)
	}
}

func TestBackgroundLighting(t *testing.T) {
	rng := rand.New(rand.NewSource(3))
	d := NewBackground()
	warm(t, d, rng)

	// the light goes on: every pixel brightens a lot, the scene is still
	for i := 0; i < 30; i++ {
		if s, _ := d.Score(scene(rng, 0, 0, 0, 60), nil); s != 0 {
			t.Fatalf("lighting change frame %d scored %v", i, s)
		}
	}
	// and the detector works again afterwards
	var s float64
	for i := 0; i < 5; i++ {
		s, _ = d.Score(scene(rng, 300, 200, 40, 60), nil)
	}
	if s == 0 {
		t.Fatal("motion after the lighting change not detected")
	}
}

func TestBackgroundStillObjectAbsorbed(t *testing.T) {
	rng := rand.New(rand.NewSource(4))
	d := NewBackground()
	warm(t, d, rng)

	// someone walks in and stands still: detected for a while, then taken
	// into the scene instead of setting off alarms for a minute
	for i := 0; i < bgStill-bgWindow; i++ {
		if s, _ := d.Score(scene(rng, 300, 200, 40, 0), nil); i >= 5 && s == 0 {
			t.Fatalf("still object forgotten after %d frames", i)
		}
	}
	for i := 0; i < 2*bgWindow; i++ {
		d.Score(scene(rng, 300, 200, 40, 0), nil)
	}
	if s, _ := d.Score(scene(rng, 300, 200, 40, 0), nil); s != 0 {
		t.Fatalf("still object scored %v after %d frames", s, bgStill+2*bgWindow)
	}

	// it walks away: the spot it left is taken into the scene too
	for i := 0; i < bgStill+2*bgWindow; i++ {
		d.Score(scene(rng, 0, 0, 0, 0), nil)
	}
	if s, _ := d.Score(scene(rng, 0, 0, 0, 0), nil); s != 0 {
		t.Fatalf("empty spot scored %v", s)
	}

	// and new motion is still detected
	var s float64
	for i := 0; i < 5; i++ {
		s, _ = d.Score(scene(rng, 100+4*i, 100, 40, 0), nil)
	}
	if s == 0 {
		t.Fatal("motion after the still object not detected")
	}
}

// A big object moving about for a long time teaches the scene a little of
// itself; once it is gone the score must drop soon, not after a minute.
func TestBackgroundNoGhostAfterActivity(t *testing.T) {
	rng := rand.New(rand.NewSource(6))
	d := NewBackground()
	warm(t, d, rng)
	for i := 0; i < 300; i++ {
		d.Score(scene(rng, 100+(i%40)*2, 100, 200, 0), nil)
	}
	last := -1
	for i := 0; i < 300; i++ {
		if s, _ := d.Score(scene(rng, 0, 0, 0, 0), nil); s > 0 {
			last = i
		}
	}
	if last >= bgStill+bgWindow {
		t.Fatalf("empty scene still scored after %d frames", last)
	}
}

func TestBackgroundReset(t *testing.T) {
	rng := rand.New(rand.NewSource(5))
	d := NewBackground()
	warm(t, d, rng)
	d.Reset()
	// the first frames after a reset only learn
	for i := 0; i < bgSettle+1; i++ {
		if s, _ := d.Score(scene(rng, 300, 200, 40, 0), nil); s != 0 {
			t.Fatalf("frame %d after reset scored %v", i, s)
		}
	}
}

func TestBackgroundYCbCr(t *testing.T) {
	d := NewBackground()
	frame := func(obj bool) image.Image {
		img := image.NewYCbCr(image.Rect(0, 0, 640, 480), image.YCbCrSubsampleRatio420)
		for i := range img.Y {
			img.Y[i] = 120
		}
		if obj {
			for y := 100; y < 160; y++ {
				for x := 100; x < 160; x++ {
					img.Y[y*img.YStride+x] = 20
				}
			}
		}
		return img
	}
	for i := 0; i < 30; i++ {
		d.Score(frame(false), nil)
	}
	var s float64
	for i := 0; i < 5; i++ {
		s, _ = d.Score(frame(true), nil)
	}
	// 60x60 source pixels are 30x30 of the 320x240 detection image
	if want := 900.0 / (320 * 240); s < want*0.8 || s > want*1.5 {
		t.Fatalf("score %v, want about %v", s, want)
	}
}

func TestFrameDifferenceWindow(t *testing.T) {
	black, white := solid(color.Black), solid(color.White)
	history := []image.Image{black, white, white, white}
	n := 0
	d := &FrameDifference{Frames: func() int { return n }}

	// whole history: black→white once in 4 pairs
	if s, _ := d.Score(white, history); s != 0.25 {
		t.Errorf("whole history: got %v want 0.25", s)
	}
	n = 3 // white, white, white
	if s, _ := d.Score(white, history); s != 0 {
		t.Errorf("last 3 frames: got %v want 0", s)
	}
	if s, err := d.Score(white, nil); s != 0 || err != nil {
		t.Errorf("no history: got %v, %v", s, err)
	}
}

func BenchmarkBackground720p(b *testing.B) {
	img := image.NewYCbCr(image.Rect(0, 0, 1280, 720), image.YCbCrSubsampleRatio420)
	d := NewBackground()
	for i := 0; i < b.N; i++ {
		d.Score(img, nil)
	}
}

func BenchmarkBackgroundRGBA640(b *testing.B) {
	img := image.NewRGBA(image.Rect(0, 0, 640, 480))
	d := NewBackground()
	for i := 0; i < b.N; i++ {
		d.Score(img, nil)
	}
}

func BenchmarkDifference30x640(b *testing.B) {
	frames := make([]image.Image, 30)
	for i := range frames {
		frames[i] = image.NewRGBA(image.Rect(0, 0, 640, 480))
	}
	for i := 0; i < b.N; i++ {
		Difference(frames)
	}
}
