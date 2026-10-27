package main

import (
	"fmt"
	"image"
	"security_camera/lib/gif"
	"security_camera/lib/storage"

	"github.com/google/uuid"
	"github.com/pion/mediadevices"
	_ "github.com/pion/mediadevices/pkg/driver/camera" // registers the camera driver
	"github.com/pion/mediadevices/pkg/frame"
	"github.com/pion/mediadevices/pkg/prop"
)

func main() {
	stream, err := mediadevices.GetUserMedia(mediadevices.MediaStreamConstraints{
		Video: func(c *mediadevices.MediaTrackConstraints) {
			c.FrameFormat = prop.FrameFormat(frame.FormatYUY2)
			c.Width = prop.Int(640)
			c.Height = prop.Int(480)
		},
	})
	if err != nil {
		panic(err)
	}

	track := stream.GetVideoTracks()[0].(*mediadevices.VideoTrack)
	defer track.Close()

	reader := track.NewReader(false)

	images := []image.Image{}

	// let auto-exposure settle (~1s at 30fps)
	for i := 0; i < 30; i++ {
		img, rel, err := reader.Read()
		if err != nil {
			panic(err)
		}
		images = append(images, img)
		rel()
	}

	localStorage, err := storage.Init("/Users/michelesaladino/Documents/code/security-camera/temp")
	if err != nil {
		panic(err)
	}

	randomUUID := uuid.New().String()

	gifImage, err := gif.Encode(images, 1)
	if err != nil {
		panic(err)
	}

	localStorage.Save(fmt.Sprintf("%s.jpeg", randomUUID), gifImage)
}
