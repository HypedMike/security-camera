package main

import (
	"context"
	"errors"
	"fmt"
	"image"
	"os"
	"os/signal"
	"time"

	"security_camera/lib/detection"
	"security_camera/lib/my_video"
	"security_camera/lib/streaming"
)

func main() {
	video, err := my_video.Open(my_video.DefaultConfig)
	if err != nil {
		panic(err)
	}
	defer video.Close()

	stream := streaming.New(video.Track(), streaming.Options{History: 50})

	stream.AddHandler(func(frame image.Image, history []image.Image, ts time.Time) {
		if len(history) == 0 {
			return
		}
		diff, err := detection.Difference(append(history, frame))
		if err != nil {
			fmt.Fprintln(os.Stderr, "detection:", err)
			return
		}
		fmt.Printf("%s difference: %.4f\n", ts.Format("15:04:05.000"), diff)
	})

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	if err := stream.Run(ctx); err != nil && !errors.Is(err, context.Canceled) {
		panic(err)
	}
}
