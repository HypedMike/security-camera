package my_video

import (
	"errors"

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

// Devices lists the cameras currently available, in driver order.
func Devices() []Device {
	var devs []Device
	for _, d := range mediadevices.EnumerateDevices() {
		if d.Kind == mediadevices.VideoInput {
			devs = append(devs, Device{ID: d.DeviceID, Label: d.Label})
		}
	}
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

// Track returns the underlying video track, e.g. for streaming.New.
func (v *Video) Track() *mediadevices.VideoTrack {
	return v.track
}

func (v *Video) Close() error {
	return v.track.Close()
}
