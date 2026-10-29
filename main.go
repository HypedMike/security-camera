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

	"security_camera/lib/config"
	"security_camera/lib/detection"
	"security_camera/lib/env"
	"security_camera/lib/gif"
	"security_camera/lib/livegraph"
	"security_camera/lib/monitor"
	"security_camera/lib/my_video"
	"security_camera/lib/streaming"
	"security_camera/lib/sysinfo"
	"security_camera/lib/telegram"
)

func main() {
	fetchCamera := flag.Bool("fetch-camera", false, "list the cameras again and choose one, even if one is already saved")
	flag.Parse()

	if err := env.Load(config.EnvFile); err != nil {
		panic(err)
	}
	err := env.Ensure(config.EnvFile, os.Stdin, os.Stdout,
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

	printDiffs := env.Bool("PRINT_DIFFERENCES")
	if env.Bool("PRINT_TELEGRAM_MESSAGES") {
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

	cfg := my_video.DefaultConfig
	if cfg.DeviceID, err = my_video.Pick(*fetchCamera); err != nil {
		panic(err)
	}
	mon := monitor.New(cfg.DeviceID, threshold, config.AlarmCooldown)
	video, err := my_video.Open(cfg)
	if err != nil {
		panic(err)
	}
	defer video.Close()

	graph := livegraph.New()
	go func() {
		if err := graph.ListenAndServe(config.GraphAddr); err != nil {
			panic(err)
		}
	}()
	fmt.Println("live graph at http://localhost" + config.GraphAddr)

	stream := streaming.New(video.Track(), streaming.Options{History: config.StreamHistory})

	// /info reports the state of the machine and of the program
	bot.AddListener(func(m telegram.Message) {
		if m.Command() != "info" {
			return
		}
		// the report takes a moment to measure, don't hold up the other messages
		go func() {
			report := sysinfo.Report(".") + "\n\n" + mon.Report()
			if err := bot.SendMessage(report); err != nil {
				fmt.Fprintln(os.Stderr, "info:", err)
			}
		}()
	})

	stream.AddHandler(func(_ image.Image, _ []image.Image, ts time.Time) {
		mon.Frame(ts)
	})

	stream.AddHandler(func(frame image.Image, _ []image.Image, ts time.Time) {
		if !snapRequested.CompareAndSwap(true, false) {
			return
		}
		// encode now: the frame is only valid until this handler returns
		var buf bytes.Buffer
		err := jpeg.Encode(&buf, frame, &jpeg.Options{Quality: config.SnapJPEGQuality})
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
		if diff >= threshold && ts.Sub(mon.LastAlarm()) > config.AlarmCooldown {
			mon.Alarm(ts)
			caption := fmt.Sprintf("Alarm: motion detected at %s (difference %.4f)", ts.Format("15:04:05"), diff)
			// encode now: the frames are only valid until this handler returns
			animation, err := gif.Encode(append(history, frame), config.GIFFrameDelay)
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
