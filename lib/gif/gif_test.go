package gif

import (
	"bytes"
	"image"
	stdgif "image/gif"
	"testing"
)

func TestEncodeShrinks(t *testing.T) {
	frames := []image.Image{
		image.NewYCbCr(image.Rect(0, 0, 640, 480), image.YCbCrSubsampleRatio420),
		image.NewYCbCr(image.Rect(0, 0, 640, 480), image.YCbCrSubsampleRatio420),
	}
	data, err := Encode(Shrink(frames, 320), 10, false)
	if err != nil {
		t.Fatal(err)
	}
	g, err := stdgif.DecodeAll(bytes.NewReader(data))
	if err != nil {
		t.Fatal(err)
	}
	if len(g.Image) != 2 {
		t.Fatalf("got %d frames, want 2", len(g.Image))
	}
	if got := g.Image[0].Bounds().Size(); got != image.Pt(320, 240) {
		t.Fatalf("got size %v, want 320x240", got)
	}
}

func TestShrinkKeepsSmallImages(t *testing.T) {
	img := image.NewRGBA(image.Rect(0, 0, 200, 100))
	if got := shrink(img, 320).Bounds().Size(); got != image.Pt(200, 100) {
		t.Fatalf("got size %v, want 200x100", got)
	}
}
