package main

import (
	"bytes"
	"context"
	"errors"
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

func main() {
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

	video, err := my_video.Open(my_video.DefaultConfig)
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

	if err := stream.Run(ctx); err != nil && !errors.Is(err, context.Canceled) {
		panic(err)
	}
}
