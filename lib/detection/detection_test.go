package detection

import (
	"image"
	"image/color"
	"image/draw"
	"testing"
)

func solid(c color.Color) image.Image {
	img := image.NewRGBA(image.Rect(0, 0, 4, 4))
	draw.Draw(img, img.Bounds(), image.NewUniform(c), image.Point{}, draw.Src)
	return img
}

func TestDifference(t *testing.T) {
	black, white := solid(color.Black), solid(color.White)

	if d, _ := Difference([]image.Image{black, black}); d != 0 {
		t.Errorf("identical: got %v want 0", d)
	}
	if d, _ := Difference([]image.Image{black, white}); d != 1 {
		t.Errorf("black/white: got %v want 1", d)
	}
	if d, _ := Difference([]image.Image{black, white, white}); d != 0.5 {
		t.Errorf("black/white/white: got %v want 0.5", d)
	}
	if _, err := Difference([]image.Image{black}); err == nil {
		t.Error("expected error for single image")
	}
	other := image.NewRGBA(image.Rect(0, 0, 2, 2))
	if _, err := Difference([]image.Image{black, other}); err == nil {
		t.Error("expected error for mismatched bounds")
	}
}
