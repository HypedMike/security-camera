package main

import (
	"bytes"
	"cmp"
	"context"
	"errors"
	"flag"
	"fmt"
	"image"
	"image/jpeg"
	"maps"
	"math"
	"os"
	"os/signal"
	"slices"
	"strconv"
	"strings"
	"time"

	"security_camera/lib/config"
	"security_camera/lib/detection"
	"security_camera/lib/env"
	"security_camera/lib/gif"
	"security_camera/lib/livegraph"
	"security_camera/lib/monitor"
	"security_camera/lib/my_video"
	"security_camera/lib/streaming"
	"security_camera/lib/telegram"
	"security_camera/lib/update"
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

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	// ALARM_THRESHOLD is the name the difference threshold had before there
	// were several detectors
	thresholds := map[string]float64{
		detection.NameBackground: envThreshold("BACKGROUND_THRESHOLD", config.BackgroundThreshold),
		detection.NameDifference: envThreshold("DIFFERENCE_THRESHOLD", envThreshold("ALARM_THRESHOLD", config.DifferenceThreshold)),
	}
	detectorName := cmp.Or(os.Getenv("DETECTOR"), config.DefaultDetector)

	cfg := my_video.DefaultConfig
	if cfg.DeviceID, err = my_video.Pick(*fetchCamera); err != nil {
		panic(err)
	}
	mon := monitor.New(cfg.DeviceID, detectorName, thresholds, config.AlarmCooldown)
	mon.SetDetectionFrames(config.DetectionFrames)
	mon.SetGIFFrames(config.GIFFrames)

	detectors := map[string]detection.Detector{}
	for _, d := range []detection.Detector{
		detection.NewBackground(),
		&detection.FrameDifference{Frames: mon.DetectionFrames},
	} {
		detectors[d.Name()] = d
	}
	if _, ok := detectors[detectorName]; !ok {
		panic(fmt.Errorf("DETECTOR: unknown detector %q, use one of: %s", detectorName, strings.Join(slices.Sorted(maps.Keys(detectors)), ", ")))
	}
	video, err := my_video.Open(cfg)
	if err != nil {
		panic(err)
	}
	defer video.Close()

	// the live graph is only served when LIVE_STREAM_GRAPH is set
	var graph *livegraph.Graph
	if env.Bool("LIVE_STREAM_GRAPH") {
		graph = livegraph.New()
		go func() {
			if err := graph.ListenAndServe(config.GraphAddr); err != nil {
				panic(err)
			}
		}()
		fmt.Println("live graph at http://localhost" + config.GraphAddr)
	}

	stream := streaming.New(video.Track(), streaming.Options{History: config.StreamHistory})

	var cmds commandState
	initTelegramCommands(ctx, cancel, bot, mon, stream, detectors, &cmds)

	stream.AddHandler(func(_ image.Image, _ []image.Image, ts time.Time) {
		mon.Frame(ts)
	})

	stream.AddHandler(func(frame image.Image, _ []image.Image, ts time.Time) {
		if !cmds.snapRequested.CompareAndSwap(true, false) {
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

	// detector is only used by this handler; /detector switches it through
	// the monitor, and the newly chosen one starts learning from scratch
	var detector detection.Detector
	stream.AddHandler(func(frame image.Image, history []image.Image, ts time.Time) {
		if name := mon.Detector(); detector == nil || detector.Name() != name {
			detector = detectors[name]
			detector.Reset()
		}
		score, err := detector.Score(frame, history)
		if err != nil {
			fmt.Fprintln(os.Stderr, "detection:", err)
			return
		}
		if graph != nil {
			graph.Push(score)
		}
		if !mon.Paused() && score >= mon.ThresholdOf(detector.Name()) && ts.Sub(mon.LastAlarm()) > config.AlarmCooldown {
			mon.Alarm(ts)
			caption := fmt.Sprintf("Alarm: motion detected at %s (%s score %.4f)", ts.Format("15:04:05"), detector.Name(), score)
			// encode now: the frames are only valid until this handler returns
			animation, err := gif.Encode(lastFrames(history, frame, mon.GIFFrames()), config.GIFFrameDelay)
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
			fmt.Printf("%s %s score: %.4f\n", ts.Format("15:04:05.000"), detector.Name(), score)
		}
	})

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

	if cmds.restart.Load() {
		// release the camera so the new process can open it
		video.Close()
		err := update.Restart()
		fmt.Fprintln(os.Stderr, "restart:", err)
		os.Exit(1)
	}
}

// envThreshold returns the alarm threshold in the variable key, or def if it
// is not set.
func envThreshold(key string, def float64) float64 {
	v := os.Getenv(key)
	if v == "" {
		return def
	}
	t, err := strconv.ParseFloat(v, 64)
	if err != nil || math.IsNaN(t) || t < 0 || t > 1 {
		panic(fmt.Errorf("%s: %q is not a number between 0 and 1", key, v))
	}
	return t
}

// lastFrames returns the last n frames of history followed by frame, so n
// counts frame itself. It returns fewer if the history is shorter.
func lastFrames(history []image.Image, frame image.Image, n int) []image.Image {
	if n < 1 {
		n = 1
	}
	if len(history) > n-1 {
		history = history[len(history)-(n-1):]
	}
	out := make([]image.Image, 0, len(history)+1)
	out = append(out, history...)
	return append(out, frame)
}
