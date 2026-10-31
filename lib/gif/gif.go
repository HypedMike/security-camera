package gif

import (
	"bytes"
	"errors"
	"image"
	"image/color"
	"image/color/palette"
	"image/draw"
	stdgif "image/gif"
	"sync"
	"sync/atomic"
)

// useRust is set by UseRust.
var useRust atomic.Bool

// UseRust picks the implementation of Shrink and Encode: the one in Rust
// (rust-lib/gif/) or, by default, the one in Go. They give the same pictures.
// It fails in a build without -tags rust.
func UseRust(on bool) error {
	if on && !RustAvailable {
		return errors.New("gif: this build has no Rust encoder (build with -tags rust)")
	}
	useRust.Store(on)
	return nil
}

// Encode builds an animated GIF from images. delay is the pause between
// frames in hundredths of a second (e.g. 10 = 100ms). dither spreads the
// palette error over neighbouring pixels: smoother colours, but the noise
// compresses badly and the file gets bigger.
func Encode(images []image.Image, delay int, dither bool) ([]byte, error) {
	if useRust.Load() {
		return encodeRust(images, delay, dither)
	}
	return encodeGo(images, delay, dither)
}

func encodeGo(images []image.Image, delay int, dither bool) ([]byte, error) {
	if len(images) == 0 {
		return nil, errors.New("gif: no images")
	}

	out := &stdgif.GIF{}
	for _, img := range images {
		// GIF frames are paletted, so quantize each frame to the palette
		var paletted *image.Paletted
		if rgba, ok := img.(*image.RGBA); ok && !dither {
			paletted = quantize(rgba)
		} else {
			paletted = image.NewPaletted(img.Bounds(), palette.Plan9)
			drawer := draw.Drawer(draw.Src)
			if dither {
				drawer = draw.FloydSteinberg
			}
			drawer.Draw(paletted, img.Bounds(), img, img.Bounds().Min)
		}
		out.Image = append(out.Image, paletted)
		out.Delay = append(out.Delay, delay)
	}

	var buf bytes.Buffer
	if err := stdgif.EncodeAll(&buf, out); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

// Shrink returns copies of images scaled down by the smallest integer factor
// that makes them at most width wide, averaging each block of pixels. The
// copies stay valid after the frame buffers are reused. A width of 0 keeps
// the original size.
func Shrink(images []image.Image, width int) []image.Image {
	out := make([]image.Image, len(images))
	for i, img := range images {
		if useRust.Load() {
			out[i] = shrinkRust(img, width)
		} else {
			out[i] = shrink(img, width)
		}
	}
	return out
}

func shrink(img image.Image, width int) *image.RGBA {
	b := img.Bounds()
	f := 1
	if width > 0 {
		f = max(1, (b.Dx()+width-1)/width)
	}
	out := image.NewRGBA(image.Rect(0, 0, b.Dx()/f, b.Dy()/f))
	n := uint32(f * f)
	for y := range out.Rect.Dy() {
		for x := range out.Rect.Dx() {
			var r, g, bl uint32
			for dy := range f {
				for dx := range f {
					cr, cg, cb, _ := img.At(b.Min.X+x*f+dx, b.Min.Y+y*f+dy).RGBA()
					r, g, bl = r+cr, g+cg, bl+cb
				}
			}
			out.SetRGBA(x, y, color.RGBA{uint8(r / n >> 8), uint8(g / n >> 8), uint8(bl / n >> 8), 0xff})
		}
	}
	return out
}

// plan9Index maps a colour reduced to 5 bits per channel to the closest
// Plan9 palette entry, so each pixel is a table lookup instead of a search
// through the 256 colours.
var plan9Index = sync.OnceValue(func() []uint8 {
	lut := make([]uint8, 1<<15)
	p := color.Palette(palette.Plan9)
	for i := range lut {
		r, g, b := uint8(i>>10)<<3|4, uint8(i>>5&31)<<3|4, uint8(i&31)<<3|4
		lut[i] = uint8(p.Index(color.RGBA{r, g, b, 0xff}))
	}
	return lut
})

func quantize(img *image.RGBA) *image.Paletted {
	lut := plan9Index()
	out := image.NewPaletted(img.Rect, palette.Plan9)
	for y := img.Rect.Min.Y; y < img.Rect.Max.Y; y++ {
		src := img.Pix[img.PixOffset(img.Rect.Min.X, y):]
		dst := out.Pix[out.PixOffset(out.Rect.Min.X, y):]
		for x := range img.Rect.Dx() {
			r, g, b := src[4*x]>>3, src[4*x+1]>>3, src[4*x+2]>>3
			dst[x] = lut[int(r)<<10|int(g)<<5|int(b)]
		}
	}
	return out
}
