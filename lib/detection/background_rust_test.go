//go:build rust

package detection

import (
	"math"
	"math/rand"
	"testing"
)

// The Rust detector must agree with the Go one on the same frames.
func TestBackgroundRustMatchesGo(t *testing.T) {
	rng := rand.New(rand.NewSource(7))
	g, r := NewBackground(), NewBackgroundRust()
	for i := 0; i < 80; i++ {
		x, size := 0, 0
		if i >= 40 {
			x, size = 300+3*i, 40
		}
		frame := scene(rng, x, 200, size, 0)
		want, err := g.Score(frame, nil)
		if err != nil {
			t.Fatal(err)
		}
		got, err := r.Score(frame, nil)
		if err != nil {
			t.Fatal(err)
		}
		if math.Abs(got-want) > 1e-4 {
			t.Fatalf("frame %d: rust %v, go %v", i, got, want)
		}
	}
}

func BenchmarkBackgroundRust720p(b *testing.B) {
	img := scene(rand.New(rand.NewSource(1)), 100, 100, 60, 0)
	d := NewBackgroundRust()
	for i := 0; i < b.N; i++ {
		d.Score(img, nil)
	}
}
