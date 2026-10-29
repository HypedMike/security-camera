package detection

import (
	"errors"
	"image"
	"sort"
)

// Background detects motion against a learned model of the empty scene.
//
// Each frame is shrunk to a small grayscale image and lightly blurred. Every
// pixel keeps a running mean and variance of its brightness; a pixel has
// changed when it is further from its mean than K standard deviations (and
// at least MinDiff). Isolated changed pixels are dropped as noise and the
// rest are grouped into connected blobs. The score of a frame is the area of
// its largest blob as a fraction of the frame, so a small object moving far
// from the camera scores the same as a small object anywhere, however still
// the rest of the scene is.
//
// When more than Lighting of the frame changes at once (a light switched on,
// the camera adjusting its exposure) the frame scores 0 and the model is
// learned again for Settle frames. The returned score is the Persist-th
// largest of the last Window frame scores, so a score at or above a
// threshold means Persist of the last Window frames reached it.
type Background struct {
	// Width is the width frames are shrunk to before detection.
	Width int
	// K is how many standard deviations from its mean a pixel must be to count as changed.
	K float64
	// MinDiff is the smallest brightness change (0 to 255) that counts as changed.
	MinDiff float64
	// Alpha is how fast unchanged pixels learn the scene (0 to 1 per frame).
	Alpha float64
	// SlowAlpha is how fast changed pixels learn the scene, so an object
	// that stops moving becomes part of the background only slowly (at the
	// default, after about 900 frames).
	SlowAlpha float64
	// Lighting is the fraction of changed pixels above which the frame is
	// taken as a lighting change instead of motion.
	Lighting float64
	// Settle is how many frames the model is learned for, without scoring,
	// at the start and after a lighting change.
	Settle int
	// Persist and Window: see the type documentation.
	Persist, Window int

	w, h, scale int
	sums        []uint32
	gray, tmp   []float32
	mean, vari  []float32
	changed     []bool
	keep        []bool
	stack       []int32
	settle      int
	scores      []float64
	next        int
}

// NewBackground returns a Background detector with default settings.
func NewBackground() *Background {
	return &Background{
		Width:     320,
		K:         3,
		MinDiff:   12,
		Alpha:     0.02,
		SlowAlpha: 0.002,
		Lighting:  0.5,
		Settle:    10,
		Persist:   3,
		Window:    5,
	}
}

const (
	settleAlpha  = 0.3
	initVariance = 25 // (5 brightness levels)²
	minVariance  = 4
)

func (d *Background) Name() string { return NameBackground }

// Reset forgets the learned scene: the next frame starts learning it again.
func (d *Background) Reset() { d.w, d.h = 0, 0 }

func (d *Background) Score(frame image.Image, _ []image.Image) (float64, error) {
	d.load(frame)
	if len(d.gray) == 0 {
		return 0, errors.New("detection: frame is empty")
	}
	if d.mean == nil {
		d.start()
		return d.push(0), nil
	}
	n := len(d.gray)

	if d.settle > 0 {
		d.settle--
		for i, g := range d.gray {
			d.learn(i, g, settleAlpha)
		}
		return d.push(0), nil
	}

	k2 := float32(d.K * d.K)
	min2 := float32(d.MinDiff * d.MinDiff)
	count := 0
	for i, g := range d.gray {
		diff := g - d.mean[i]
		d2 := diff * diff
		c := d2 > k2*d.vari[i] && d2 > min2
		d.changed[i] = c
		if c {
			count++
		}
	}

	if float64(count) > d.Lighting*float64(n) {
		d.settle = d.Settle
		for i, g := range d.gray {
			d.learn(i, g, settleAlpha)
		}
		return d.push(0), nil
	}

	alpha, slow := float32(d.Alpha), float32(d.SlowAlpha)
	for i, g := range d.gray {
		if d.changed[i] {
			// only the mean: learning the variance from the object itself
			// would hide it within seconds
			d.mean[i] += slow * (g - d.mean[i])
		} else {
			d.learn(i, g, alpha)
		}
	}

	d.despeckle()
	return d.push(float64(d.largestBlob()) / float64(n)), nil
}

// load shrinks frame into d.gray, reallocating (and so resetting the model)
// when the frame size or Width changed.
func (d *Background) load(frame image.Image) {
	b := frame.Bounds()
	width := max(d.Width, 1)
	scale := max((b.Dx()+width-1)/width, 1)
	w, h := b.Dx()/scale, b.Dy()/scale
	if w != d.w || h != d.h || scale != d.scale {
		d.w, d.h, d.scale = w, h, scale
		n := w * h
		d.sums = make([]uint32, n)
		d.gray = make([]float32, n)
		d.tmp = make([]float32, n)
		d.changed = make([]bool, n)
		d.keep = make([]bool, n)
		d.mean, d.vari = nil, nil
	}
	if w == 0 || h == 0 {
		return
	}

	clear(d.sums)
	rows, cols := h*scale, w*scale
	switch img := frame.(type) {
	case *image.YCbCr:
		// the Y plane already is the brightness
		for y := 0; y < rows; y++ {
			row := img.Y[y*img.YStride:]
			out := d.sums[(y/scale)*w:]
			for x := 0; x < cols; x++ {
				out[x/scale] += uint32(row[x])
			}
		}
	case *image.RGBA:
		for y := 0; y < rows; y++ {
			row := img.Pix[y*img.Stride:]
			out := d.sums[(y/scale)*w:]
			for x := 0; x < cols; x++ {
				p := row[x*4 : x*4+3]
				out[x/scale] += luma(uint32(p[0]), uint32(p[1]), uint32(p[2]))
			}
		}
	default:
		for y := 0; y < rows; y++ {
			out := d.sums[(y/scale)*w:]
			for x := 0; x < cols; x++ {
				r, g, bl, _ := frame.At(b.Min.X+x, b.Min.Y+y).RGBA()
				out[x/scale] += luma(r>>8, g>>8, bl>>8)
			}
		}
	}

	// average each cell, then a 3x3 box blur to quiet the sensor noise
	area := float32(scale * scale)
	for i, s := range d.sums {
		d.tmp[i] = float32(s) / area
	}
	for y := 0; y < h; y++ {
		for x := 0; x < w; x++ {
			var sum float32
			var cnt int
			for yy := max(y-1, 0); yy <= min(y+1, h-1); yy++ {
				for xx := max(x-1, 0); xx <= min(x+1, w-1); xx++ {
					sum += d.tmp[yy*w+xx]
					cnt++
				}
			}
			d.gray[y*w+x] = sum / float32(cnt)
		}
	}
}

// luma returns the brightness (0 to 255) of 8-bit R, G, B values (BT.601).
func luma(r, g, b uint32) uint32 {
	return (19595*r + 38470*g + 7471*b + 1<<15) >> 16
}

// start begins learning the scene from the current frame.
func (d *Background) start() {
	n := len(d.gray)
	d.mean = make([]float32, n)
	d.vari = make([]float32, n)
	copy(d.mean, d.gray)
	for i := range d.vari {
		d.vari[i] = initVariance
	}
	d.settle = d.Settle
	d.scores = make([]float64, max(d.Window, 1))
	d.next = 0
}

func (d *Background) learn(i int, g, alpha float32) {
	diff := g - d.mean[i]
	d.mean[i] += alpha * diff
	d.vari[i] = max(d.vari[i]+alpha*(diff*diff-d.vari[i]), minVariance)
}

// despeckle drops changed pixels with fewer than two changed neighbours.
func (d *Background) despeckle() {
	w, h := d.w, d.h
	for y := 0; y < h; y++ {
		for x := 0; x < w; x++ {
			i := y*w + x
			if !d.changed[i] {
				d.keep[i] = false
				continue
			}
			nb := 0
			for yy := max(y-1, 0); yy <= min(y+1, h-1); yy++ {
				for xx := max(x-1, 0); xx <= min(x+1, w-1); xx++ {
					if d.changed[yy*w+xx] {
						nb++
					}
				}
			}
			d.keep[i] = nb-1 >= 2
		}
	}
	copy(d.changed, d.keep)
}

// largestBlob returns the pixel count of the largest 8-connected group of
// changed pixels. It clears d.changed.
func (d *Background) largestBlob() int {
	w, h := d.w, d.h
	largest := 0
	for start, c := range d.changed {
		if !c {
			continue
		}
		d.changed[start] = false
		d.stack = append(d.stack[:0], int32(start))
		size := 0
		for len(d.stack) > 0 {
			i := int(d.stack[len(d.stack)-1])
			d.stack = d.stack[:len(d.stack)-1]
			size++
			x, y := i%w, i/w
			for yy := max(y-1, 0); yy <= min(y+1, h-1); yy++ {
				for xx := max(x-1, 0); xx <= min(x+1, w-1); xx++ {
					if j := yy*w + xx; d.changed[j] {
						d.changed[j] = false
						d.stack = append(d.stack, int32(j))
					}
				}
			}
		}
		largest = max(largest, size)
	}
	return largest
}

// push records a frame score and returns the Persist-th largest of the last
// Window scores.
func (d *Background) push(s float64) float64 {
	if len(d.scores) == 0 {
		return s
	}
	d.scores[d.next] = s
	d.next = (d.next + 1) % len(d.scores)
	sorted := append([]float64(nil), d.scores...)
	sort.Sort(sort.Reverse(sort.Float64Slice(sorted)))
	p := min(max(d.Persist, 1), len(sorted))
	return sorted[p-1]
}
