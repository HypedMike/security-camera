package main

import (
	"bytes"
	"context"
	"errors"
	"flag"
	"fmt"
	"image"
	"image/jpeg"
	"os"
	"os/signal"
	"strconv"
	"sync/atomic"
	"time"

	"security_camera/lib/detection"
	"security_camera/lib/env"
	"security_camera/lib/gif"
	"security_camera/lib/livegraph"
	"security_camera/lib/my_video"
	"security_camera/lib/streaming"
	"security_camera/lib/telegram"
)

// alarmCooldown is the minimum time between two Telegram notifications.
const alarmCooldown = 30 * time.Second

// gifFrameDelay is the pause between GIF frames, in hundredths of a second.
const gifFrameDelay = 10

// envBool reports whether the variable is set to a true value (1, true, ...).
func envBool(key string) bool {
	v, _ := strconv.ParseBool(os.Getenv(key))
	return v
}

// cameraChoiceTimeout is how long the camera prompt waits before using the first camera.
const cameraChoiceTimeout = 5 * time.Second

// pickCamera returns the id of the camera to use. The choice is kept in
// CAMERA_ID; it is asked again only when refetch is set, nothing is saved yet
// or the saved camera is gone. With a single camera there is nothing to ask.
func pickCamera(refetch bool) (string, error) {
	devs := my_video.Devices()
	switch len(devs) {
	case 0:
		// let Open try the default camera and report the driver's own error
		fmt.Fprintln(os.Stderr, "no camera listed, trying the default one")
		return "", nil
	case 1:
		return devs[0].ID, nil
	}
	if saved := os.Getenv("CAMERA_ID"); saved != "" && !refetch {
		for _, d := range devs {
			if d.ID == saved {
				return saved, nil
			}
		}
		fmt.Println("saved camera not found, choosing again")
	}
	labels := make([]string, len(devs))
	for i, d := range devs {
		labels[i] = d.Label
	}
	chosen := devs[env.Choose(os.Stdin, os.Stdout, "Several cameras found:", labels, cameraChoiceTimeout)]
	if err := env.Save(".env", "CAMERA_ID", chosen.ID); err != nil {
		return "", err
	}
	os.Setenv("CAMERA_ID", chosen.ID)
	return chosen.ID, nil
}

func main() {
	fetchCamera := flag.Bool("fetch-camera", false, "list the cameras again and choose one, even if one is already saved")
	flag.Parse()

	if err := env.Load(".env"); err != nil {
		panic(err)
	}
	err := env.Ensure(".env", os.Stdin, os.Stdout,
		env.Var{Key: "TELEGRAM_BOT_TOKEN", Prompt: "Telegram bot token from @BotFather"},
		env.Var{Key: "TELEGRAM_CHAT_ID", Prompt: "Telegram chat id"},
		env.Var{Key: "ALARM_THRESHOLD", Prompt: "Frame difference at or above which an alarm is sent, e.g. 0.05", Validate: func(s string) error {
			_, err := strconv.ParseFloat(s, 64)
			return err
		}},
	)
	if err != nil {
		panic(err)
	}
	bot, err := telegram.New(os.Getenv("TELEGRAM_BOT_TOKEN"), os.Getenv("TELEGRAM_CHAT_ID"))
	if err != nil {
		panic(fmt.Errorf("%w (set them in .env)", err))
	}

	printDiffs := envBool("PRINT_DIFFERENCES")
	if envBool("PRINT_TELEGRAM_MESSAGES") {
		bot.AddListener(func(m telegram.Message) {
			fmt.Printf("Telegram message from %s: %s\n", m.From, m.Text)
		})
	}

	// /snap asks the stream handler to send the next frame
	var snapRequested atomic.Bool
	bot.AddListener(func(m telegram.Message) {
		if m.Command() == "snap" {
			snapRequested.Store(true)
		}
	})

	threshold, err := strconv.ParseFloat(os.Getenv("ALARM_THRESHOLD"), 64)
	if err != nil {
		panic(fmt.Errorf("ALARM_THRESHOLD: %w", err))
	}
	var lastAlarm atomic.Int64 // unix nanos

	cfg := my_video.DefaultConfig
	if cfg.DeviceID, err = pickCamera(*fetchCamera); err != nil {
		panic(err)
	}
	video, err := my_video.Open(cfg)
	if err != nil {
		panic(err)
	}
	defer video.Close()

	graph := livegraph.New()
	go func() {
		if err := graph.ListenAndServe(":8080"); err != nil {
			panic(err)
		}
	}()
	fmt.Println("live graph at http://localhost:8080")

	stream := streaming.New(video.Track(), streaming.Options{History: 10})

	stream.AddHandler(func(frame image.Image, _ []image.Image, ts time.Time) {
		if !snapRequested.CompareAndSwap(true, false) {
			return
		}
		// encode now: the frame is only valid until this handler returns
		var buf bytes.Buffer
		err := jpeg.Encode(&buf, frame, &jpeg.Options{Quality: 85})
		go func() {
			if err == nil {
				err = bot.SendPhoto(buf.Bytes(), "Snapshot at "+ts.Format("15:04:05"))
			}
			if err != nil {
				fmt.Fprintln(os.Stderr, "snap:", err)
			}
		}()
	})

	stream.AddHandler(func(frame image.Image, history []image.Image, ts time.Time) {
		if len(history) == 0 {
			return
		}
		diff, err := detection.Difference(append(history, frame))
		if err != nil {
			fmt.Fprintln(os.Stderr, "detection:", err)
			return
		}
		graph.Push(diff)
		if diff >= threshold && ts.UnixNano()-lastAlarm.Load() > int64(alarmCooldown) {
			lastAlarm.Store(ts.UnixNano())
			caption := fmt.Sprintf("Alarm: motion detected at %s (difference %.4f)", ts.Format("15:04:05"), diff)
			// encode now: the frames are only valid until this handler returns
			animation, err := gif.Encode(append(history, frame), gifFrameDelay)
			go func() {
				if err != nil {
					fmt.Fprintln(os.Stderr, "gif:", err)
					err = bot.SendMessage(caption)
				} else {
					err = bot.SendAnimation(animation, caption)
				}
				if err != nil {
					fmt.Fprintln(os.Stderr, err)
				}
			}()
		}
		if printDiffs {
			fmt.Printf("%s difference: %.4f\n", ts.Format("15:04:05.000"), diff)
		}
	})

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	go func() {
		if err := bot.Run(ctx); err != nil && !errors.Is(err, context.Canceled) {
			fmt.Fprintln(os.Stderr, err)
		}
	}()

	// the camera is open and the stream is about to start
	go func() {
		host, _ := os.Hostname()
		if err := bot.SendMessage(fmt.Sprintf("Security camera is up and running on %s", host)); err != nil {
			fmt.Fprintln(os.Stderr, err)
		}
	}()

	if err := stream.Run(ctx); err != nil && !errors.Is(err, context.Canceled) {
		panic(err)
	}
}
