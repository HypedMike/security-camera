package detection

/*
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

	_ "security_camera/lib/native"
)

// Background detects motion against a learned model of the empty scene. It
// is implemented in Rust, see rust-lib/detection/src/lib.rs for how it works.
//
// Each frame is shrunk to a small grayscale image and compared with a
// per-pixel running mean and variance of the scene. The score of a frame is
// the area of its largest blob of changed pixels as a fraction of the frame.
// Lighting changes are ignored while the model relearns, a score must hold
// for 3 of the last 5 frames, and a change that holds still for 60 frames
// becomes part of the scene.
type Background struct {
	bg unsafe.Pointer
}

// NewBackground returns a Background detector with default settings.
func NewBackground() *Background {
	d := &Background{bg: C.bg_new()}
	runtime.SetFinalizer(d, func(d *Background) { C.bg_free(d.bg) })
	return d
}

func (d *Background) Name() string { return NameBackground }

func (d *Background) Reset() { C.bg_reset(d.bg) }

func (d *Background) Score(frame image.Image, _ []image.Image) (float64, error) {
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
