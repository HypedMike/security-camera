package detection

import "image"

// Names of the detectors, as used by the DETECTOR setting and /detector.
const (
	NameBackground     = "background"
	NameBackgroundRust = "background-rust"
	NameDifference     = "difference"
)

// Detector turns each camera frame into a motion score: 0 means no motion,
// and the higher the score the more motion. What the score measures depends
// on the detector, so every detector has its own alarm threshold.
//
// A detector may keep state between frames, so it is fed every frame in
// order and is not safe for concurrent use.
type Detector interface {
	// Name identifies the detector, e.g. NameBackground.
	Name() string
	// Score returns the motion score of frame. history holds the previous
	// frames, oldest first; detectors that keep their own state ignore it.
	Score(frame image.Image, history []image.Image) (float64, error)
	// Reset forgets the state learned so far, e.g. when the detector is
	// switched back on after frames went by without it.
	Reset()
}

// FrameDifference is the Detector built on Difference: the score is the
// average change between consecutive frames over the latest frames.
type FrameDifference struct {
	// Frames returns how many of the latest frames the difference is
	// computed on, counting the current one. nil or less than 2 uses the
	// whole history.
	Frames func() int
}

func (d *FrameDifference) Name() string { return NameDifference }

func (d *FrameDifference) Reset() {}

func (d *FrameDifference) Score(frame image.Image, history []image.Image) (float64, error) {
	if len(history) == 0 {
		return 0, nil
	}
	if d.Frames != nil {
		if n := d.Frames(); n >= 2 && n-1 < len(history) {
			history = history[len(history)-(n-1):]
		}
	}
	frames := make([]image.Image, 0, len(history)+1)
	frames = append(frames, history...)
	return Difference(append(frames, frame))
}
