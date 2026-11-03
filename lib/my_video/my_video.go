package my_video

import (
	"errors"
	"fmt"
	"os"
	"sort"

	"github.com/pion/mediadevices"
	_ "github.com/pion/mediadevices/pkg/driver/camera" // registers the camera driver
	"github.com/pion/mediadevices/pkg/frame"
	"github.com/pion/mediadevices/pkg/prop"
)

type Config struct {
	Width  int
	Height int
	// WarmupFrames are read and discarded on open so auto-exposure can settle.
	WarmupFrames int
	// DeviceID selects the camera (see Devices); empty means the default one.
	DeviceID string
}

// Device is an available camera.
type Device struct {
	ID    string
	Label string
}

// Devices lists the cameras currently available, sorted by label. The driver
// hands them out in a different random order at every start, and gives each
// one a new random id too: the label is the only thing that stays the same.
func Devices() []Device {
	var devs []Device
	for _, d := range mediadevices.EnumerateDevices() {
		if d.Kind == mediadevices.VideoInput {
			devs = append(devs, Device{ID: d.DeviceID, Label: d.Label})
		}
	}
	sort.Slice(devs, func(i, j int) bool { return devs[i].Label < devs[j].Label })
	return devs
}

var DefaultConfig = Config{Width: 640, Height: 480, WarmupFrames: 30}

// Video owns the camera stream and its video track.
type Video struct {
	stream mediadevices.MediaStream
	track  *mediadevices.VideoTrack
}

func Open(cfg Config) (*Video, error) {
	stream, err := mediadevices.GetUserMedia(mediadevices.MediaStreamConstraints{
		Video: func(c *mediadevices.MediaTrackConstraints) {
			c.FrameFormat = prop.FrameFormat(frame.FormatYUY2)
			if cfg.DeviceID != "" {
				c.DeviceID = prop.StringExact(cfg.DeviceID)
			}
			c.Width = prop.Int(cfg.Width)
			c.Height = prop.Int(cfg.Height)
		},
	})
	if err != nil {
		return nil, err
	}

	tracks := stream.GetVideoTracks()
	if len(tracks) == 0 {
		return nil, errors.New("my_video: no video track available")
	}
	track := tracks[0].(*mediadevices.VideoTrack)
	v := &Video{stream: stream, track: track}

	reader := track.NewReader(false)
	for i := 0; i < cfg.WarmupFrames; i++ {
		_, release, err := reader.Read()
		if err != nil {
			v.Close()
			return nil, err
		}
		release()
	}
	return v, nil
}

// OpenAny opens the camera cfg.DeviceID. A camera often shows up as several
// video nodes of which only one delivers images, so if that one can't be
// opened the other cameras are tried in turn, and a wrong choice does not stop
// the program. If none opens, it returns the first error.
func OpenAny(cfg Config) (*Video, error) {
	devs := Devices()
	label := map[string]string{}
	for _, d := range devs {
		label[d.ID] = d.Label
	}
	var firstErr error
	for _, id := range openOrder(devs, cfg.DeviceID) {
		c := cfg
		c.DeviceID = id
		v, err := Open(c)
		if err == nil {
			return v, nil
		}
		fmt.Fprintf(os.Stderr, "camera %q can't be opened: %v\n", describe(label[id], id), err)
		if firstErr == nil {
			firstErr = err
		}
	}
	return nil, firstErr
}

func describe(label, id string) string {
	if label != "" {
		return label
	}
	if id == "" {
		return "default"
	}
	return id
}

// openOrder returns the ids to try: first, then the others in the order of
// devs. With no camera listed it is just first (maybe empty, the default
// camera), so the driver reports its own error.
func openOrder(devs []Device, first string) []string {
	order := []string{first}
	for _, d := range devs {
		if d.ID != first {
			order = append(order, d.ID)
		}
	}
	return order
}

// Track returns the underlying video track, e.g. for streaming.New.
func (v *Video) Track() *mediadevices.VideoTrack {
	return v.track
}

func (v *Video) Close() error {
	return v.track.Close()
}
