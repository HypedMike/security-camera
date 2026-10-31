//go:build !rust

package gif

import (
	"image"
)

// RustAvailable reports whether this binary was built with the Rust
// encoder (go build -tags rust).
const RustAvailable = false

// The Rust encoder is not in this build, and UseRust refuses to switch to
// it: these are never called.
func shrinkRust(img image.Image, width int) *image.RGBA { return shrink(img, width) }

func encodeRust(images []image.Image, delay int, dither bool) ([]byte, error) {
	return encodeGo(images, delay, dither)
}
