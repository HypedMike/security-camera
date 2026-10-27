package detection

import (
	"errors"
	"image"
)

// Difference measures how much a sequence of images differs at pixel level.
// It compares each image with the next one and returns the average absolute
// per-channel (R, G, B) difference across all pixels and pairs, normalized to
// [0, 1]: 0 means identical images, 1 means every channel flipped between
// black and white. All images must have the same bounds.
func Difference(images []image.Image) (float64, error) {
	if len(images) < 2 {
		return 0, errors.New("detection: need at least 2 images")
	}

	bounds := images[0].Bounds()
	for _, img := range images[1:] {
		if img.Bounds() != bounds {
			return 0, errors.New("detection: images have different bounds")
		}
	}
	if bounds.Empty() {
		return 0, errors.New("detection: images are empty")
	}

	var total float64
	for i := 1; i < len(images); i++ {
		total += pairDifference(images[i-1], images[i], bounds)
	}
	return total / float64(len(images)-1), nil
}

func pairDifference(a, b image.Image, bounds image.Rectangle) float64 {
	var sum uint64
	for y := bounds.Min.Y; y < bounds.Max.Y; y++ {
		for x := bounds.Min.X; x < bounds.Max.X; x++ {
			r1, g1, b1, _ := a.At(x, y).RGBA()
			r2, g2, b2, _ := b.At(x, y).RGBA()
			sum += absDiff(r1, r2) + absDiff(g1, g2) + absDiff(b1, b2)
		}
	}
	// RGBA() channels are 16-bit, so the max per channel is 0xffff.
	return float64(sum) / (float64(bounds.Dx()*bounds.Dy()) * 3 * 0xffff)
}

func absDiff(a, b uint32) uint64 {
	if a > b {
		return uint64(a - b)
	}
	return uint64(b - a)
}
