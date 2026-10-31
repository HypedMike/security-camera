//go:build rust

package detection

/*
#cgo LDFLAGS: -L${SRCDIR}/../../detection-lib/target/release -ldetection_lib
#cgo linux LDFLAGS: -lm -ldl -lpthread
#include <stdint.h>
#include <stddef.h>

void *bg_new(void);
void bg_free(void *bg);
void bg_reset(void *bg);
int bg_score(void *bg, int kind, const uint8_t *data, size_t len, size_t width, size_t height, size_t stride, double *out);
*/
import "C"

import (
	"errors"
	"fmt"
	"image"
	"image/draw"
	"runtime"
	"unsafe"
)

// RustAvailable reports whether this binary was built with the Rust
// detectors (go build -tags rust).
const RustAvailable = true

// BackgroundRust is Background (same algorithm and defaults) implemented in
// Rust, see detection-lib/.
type BackgroundRust struct {
	bg unsafe.Pointer
}

// NewBackgroundRust returns a BackgroundRust detector with default settings.
func NewBackgroundRust() *BackgroundRust {
	d := &BackgroundRust{bg: C.bg_new()}
	runtime.SetFinalizer(d, func(d *BackgroundRust) { C.bg_free(d.bg) })
	return d
}

func (d *BackgroundRust) Name() string { return NameBackgroundRust }

func (d *BackgroundRust) Reset() { C.bg_reset(d.bg) }

func (d *BackgroundRust) Score(frame image.Image, _ []image.Image) (float64, error) {
	b := frame.Bounds()
	var (
		kind   C.int
		pix    []byte
		stride int
	)
	switch img := frame.(type) {
	case *image.YCbCr:
		// the Y plane already is the brightness
		kind, stride = 0, img.YStride
		if !b.Empty() {
			pix = img.Y[img.YOffset(b.Min.X, b.Min.Y):]
		}
	case *image.RGBA:
		kind, stride = 1, img.Stride
		if !b.Empty() {
			pix = img.Pix[img.PixOffset(b.Min.X, b.Min.Y):]
		}
	default:
		rgba := image.NewRGBA(image.Rect(0, 0, b.Dx(), b.Dy()))
		draw.Draw(rgba, rgba.Bounds(), frame, b.Min, draw.Src)
		kind, pix, stride = 1, rgba.Pix, rgba.Stride
	}
	if len(pix) == 0 {
		return 0, errors.New("detection: frame is empty")
	}

	var score C.double
	rc := C.bg_score(d.bg, kind, (*C.uint8_t)(unsafe.Pointer(&pix[0])), C.size_t(len(pix)),
		C.size_t(b.Dx()), C.size_t(b.Dy()), C.size_t(stride), &score)
	runtime.KeepAlive(pix)
	switch rc {
	case 0:
		return float64(score), nil
	case 1:
		return 0, errors.New("detection: frame is empty")
	default:
		return 0, fmt.Errorf("detection: rust detector failed (code %d)", rc)
	}
}
