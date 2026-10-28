package main

import (
	"context"
	"errors"
	"fmt"
	"image"
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

func main() {
	if err := env.Load(".env"); err != nil {
		panic(err)
	}
	bot, err := telegram.New(os.Getenv("TELEGRAM_BOT_TOKEN"), os.Getenv("TELEGRAM_CHAT_ID"))
	if err != nil {
		panic(fmt.Errorf("%w (set them in .env)", err))
	}

	bot.AddListener(func(m telegram.Message) {
		fmt.Printf("Telegram message from %s: %s\n", m.From, m.Text)
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
		fmt.Printf("%s difference: %.4f\n", ts.Format("15:04:05.000"), diff)
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
