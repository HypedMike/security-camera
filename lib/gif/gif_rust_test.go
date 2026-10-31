//go:build rust

package gif

import (
	"bytes"
	"image"
	stdgif "image/gif"
	"math/rand"
	"testing"
)

// frames returns n noisy 640x480 test frames: RGBA, or 4:2:2 YCbCr like the camera's.
func frames(n int, ycbcr bool) []image.Image {
	rng := rand.New(rand.NewSource(1))
	out := make([]image.Image, n)
	for i := range out {
		if ycbcr {
			img := image.NewYCbCr(image.Rect(0, 0, 640, 480), image.YCbCrSubsampleRatio422)
			for j := range img.Y {
				img.Y[j] = uint8(j%640/4 + rng.Intn(8))
			}
			for j := range img.Cb {
				img.Cb[j], img.Cr[j] = uint8(100+rng.Intn(60)), uint8(120+rng.Intn(60))
			}
			out[i] = img
			continue
		}
		img := image.NewRGBA(image.Rect(0, 0, 640, 480))
		for j := 0; j < len(img.Pix); j += 4 {
			img.Pix[j] = uint8(j/4%640/3 + rng.Intn(8))
			img.Pix[j+1] = uint8(j/4/640/2 + rng.Intn(8))
			img.Pix[j+2] = uint8(rng.Intn(256))
			img.Pix[j+3] = 255
		}
		out[i] = img
	}
	return out
}

func TestShrinkRustMatchesGo(t *testing.T) {
	for _, ycbcr := range []bool{false, true} {
		for _, width := range []int{0, 320, 160, 200} {
			in := frames(1, ycbcr)
			want := shrink(in[0], width)
			got := shrinkRust(in[0], width)
			if got.Rect != want.Rect || !bytes.Equal(got.Pix, want.Pix) {
				t.Fatalf("ycbcr=%v width=%d: shrunk images differ", ycbcr, width)
			}
		}
	}
}

func TestEncodeRustMatchesGo(t *testing.T) {
	for _, ycbcr := range []bool{false, true} {
		for _, dither := range []bool{false, true} {
			in := Shrink(frames(3, ycbcr), 320)
			want, err := encodeGo(in, 10, dither)
			if err != nil {
				t.Fatal(err)
			}
			got, err := encodeRust(in, 10, dither)
			if err != nil {
				t.Fatal(err)
			}
			wg, err := stdgif.DecodeAll(bytes.NewReader(want))
			if err != nil {
				t.Fatal(err)
			}
			gg, err := stdgif.DecodeAll(bytes.NewReader(got))
			if err != nil {
				t.Fatalf("rust output does not decode: %v", err)
			}
			if len(gg.Image) != len(wg.Image) || gg.Delay[0] != 10 {
				t.Fatalf("got %d frames, delay %v", len(gg.Image), gg.Delay)
			}
			for i := range wg.Image {
				if !bytes.Equal(gg.Image[i].Pix, wg.Image[i].Pix) {
					t.Fatalf("ycbcr=%v dither=%v: frame %d pixels differ", ycbcr, dither, i)
				}
			}
			t.Logf("ycbcr=%v dither=%v: go %d bytes, rust %d bytes", ycbcr, dither, len(want), len(got))
		}
	}
}

func benchEncode(b *testing.B, enc func([]image.Image, int, bool) ([]byte, error), dither bool) {
	in := Shrink(frames(10, true), 640)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		enc(in, 10, dither)
	}
}

func benchShrink(b *testing.B, shr func(image.Image, int) image.Image) {
	in := frames(10, true)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		for _, img := range in {
			shr(img, 320)
		}
	}
}

func BenchmarkShrinkGo(b *testing.B) {
	benchShrink(b, func(i image.Image, w int) image.Image { return shrink(i, w) })
}
func BenchmarkShrinkRust(b *testing.B) {
	benchShrink(b, func(i image.Image, w int) image.Image { return shrinkRust(i, w) })
}
func BenchmarkEncodeGo(b *testing.B)         { benchEncode(b, encodeGo, false) }
func BenchmarkEncodeRust(b *testing.B)       { benchEncode(b, encodeRust, false) }
func BenchmarkEncodeDitherGo(b *testing.B)   { benchEncode(b, encodeGo, true) }
func BenchmarkEncodeDitherRust(b *testing.B) { benchEncode(b, encodeRust, true) }

func TestUseRustSwitchesTheImplementation(t *testing.T) {
	defer UseRust(false)
	in := Shrink(frames(2, true), 320)
	if err := UseRust(true); err != nil {
		t.Fatal(err)
	}
	if _, err := Encode(in, 10, true); err != nil {
		t.Fatal(err)
	}
}
