package gif

import (
	"bytes"
	"errors"
	"image"
	"image/color/palette"
	"image/draw"
	stdgif "image/gif"
)

// Encode builds an animated GIF from images. delay is the pause between
// frames in hundredths of a second (e.g. 10 = 100ms).
func Encode(images []image.Image, delay int) ([]byte, error) {
	if len(images) == 0 {
		return nil, errors.New("gif: no images")
	}

	out := &stdgif.GIF{}
	for _, img := range images {
		// GIF frames are paletted, so quantize each frame with dithering.
		paletted := image.NewPaletted(img.Bounds(), palette.Plan9)
		draw.FloydSteinberg.Draw(paletted, img.Bounds(), img, img.Bounds().Min)
		out.Image = append(out.Image, paletted)
		out.Delay = append(out.Delay, delay)
	}

	var buf bytes.Buffer
	if err := stdgif.EncodeAll(&buf, out); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}
