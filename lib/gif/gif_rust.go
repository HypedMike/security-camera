//go:build rust

package gif

/*
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

void gif_shrink_rgba(const uint8_t *data, size_t len, size_t stride, size_t width, size_t height, size_t target, uint8_t *out);
void gif_shrink_ycbcr(const uint8_t *y, size_t ylen, const uint8_t *cb, const uint8_t *cr, size_t clen, size_t ystride, size_t cstride, uint32_t xs, uint32_t ys, size_t width, size_t height, size_t target, uint8_t *out);
void *gif_enc_new(size_t width, size_t height, uint16_t delay, bool dither);
void gif_enc_add_rgba(void *enc, const uint8_t *data, size_t len, size_t stride);
uint8_t *gif_enc_finish(void *enc, size_t *len);
void gif_free(uint8_t *ptr, size_t len);
*/
import "C"

import (
	"errors"
	"image"
	"image/draw"
	"unsafe"

	_ "security_camera/lib/native"
)

// RustAvailable reports whether this binary was built with the Rust
// encoder (go build -tags rust).
const RustAvailable = true

// shrinkRust is shrink implemented in Rust, see rust-lib/gif/.
func shrinkRust(img image.Image, width int) *image.RGBA {
	b := img.Bounds()
	f := 1
	if width > 0 {
		f = max(1, (b.Dx()+width-1)/width)
	}
	out := image.NewRGBA(image.Rect(0, 0, b.Dx()/f, b.Dy()/f))
	if out.Rect.Empty() {
		return out
	}
	w, h, target := C.size_t(b.Dx()), C.size_t(b.Dy()), C.size_t(max(width, 0))
	dst := (*C.uint8_t)(unsafe.Pointer(&out.Pix[0]))
	switch src := img.(type) {
	case *image.RGBA:
		pix := src.Pix[src.PixOffset(b.Min.X, b.Min.Y):]
		C.gif_shrink_rgba((*C.uint8_t)(unsafe.Pointer(&pix[0])), C.size_t(len(pix)), C.size_t(src.Stride), w, h, target, dst)
	case *image.YCbCr:
		var xs, ys C.uint32_t
		switch src.SubsampleRatio {
		case image.YCbCrSubsampleRatio444:
		case image.YCbCrSubsampleRatio422:
			xs = 1
		case image.YCbCrSubsampleRatio420:
			xs, ys = 1, 1
		case image.YCbCrSubsampleRatio440:
			ys = 1
		case image.YCbCrSubsampleRatio411:
			xs = 2
		case image.YCbCrSubsampleRatio410:
			xs, ys = 2, 1
		}
		y := src.Y[src.YOffset(b.Min.X, b.Min.Y):]
		co := src.COffset(b.Min.X, b.Min.Y)
		cb, cr := src.Cb[co:], src.Cr[co:]
		C.gif_shrink_ycbcr((*C.uint8_t)(unsafe.Pointer(&y[0])), C.size_t(len(y)),
			(*C.uint8_t)(unsafe.Pointer(&cb[0])), (*C.uint8_t)(unsafe.Pointer(&cr[0])), C.size_t(len(cb)),
			C.size_t(src.YStride), C.size_t(src.CStride), xs, ys, w, h, target, dst)
	default:
		return shrink(img, width)
	}
	return out
}

// encodeRust is Encode implemented in Rust, see rust-lib/gif/. Frames of
// different sizes are encoded by the Go encoder instead.
func encodeRust(images []image.Image, delay int, dither bool) ([]byte, error) {
	if len(images) == 0 {
		return nil, errors.New("gif: no images")
	}
	size := images[0].Bounds().Size()
	frames := make([]*image.RGBA, len(images))
	for i, img := range images {
		if img.Bounds().Size() != size || size.X == 0 || size.Y == 0 || size.X > 0xffff || size.Y > 0xffff {
			return encodeGo(images, delay, dither)
		}
		if rgba, ok := img.(*image.RGBA); ok {
			frames[i] = rgba
			continue
		}
		rgba := image.NewRGBA(image.Rect(0, 0, size.X, size.Y))
		draw.Draw(rgba, rgba.Bounds(), img, img.Bounds().Min, draw.Src)
		frames[i] = rgba
	}

	enc := C.gif_enc_new(C.size_t(size.X), C.size_t(size.Y), C.uint16_t(delay), C.bool(dither))
	for _, f := range frames {
		pix := f.Pix[f.PixOffset(f.Rect.Min.X, f.Rect.Min.Y):]
		C.gif_enc_add_rgba(enc, (*C.uint8_t)(unsafe.Pointer(&pix[0])), C.size_t(len(pix)), C.size_t(f.Stride))
	}
	var n C.size_t
	p := C.gif_enc_finish(enc, &n)
	out := C.GoBytes(unsafe.Pointer(p), C.int(n))
	C.gif_free(p, n)
	return out, nil
}
