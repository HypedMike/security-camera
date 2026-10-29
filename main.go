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

	// without the store the alarms still work, they just have no HD version
	gifs, err := gif.NewStore(config.GIFDir, config.GIFStorageLimit)
	if err != nil {
		fmt.Fprintln(os.Stderr, "gifs: HD GIFs will not be saved:", err)
		gifs = nil
	}

	var cmds commandState
	initTelegramCommands(ctx, cancel, bot, mon, stream, detectors, gifs, &cmds)

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
	// alarm is the detection waiting for AlarmDelay to pass, nil if none
	var alarm *pendingAlarm
	stream.AddHandler(func(frame image.Image, history []image.Image, ts time.Time) {
		if name := mon.Detector(); detector == nil || detector.Name() != name {
			detector = detectors[name]
			detector.Reset()
			alarm = nil
		}
		score, err := detector.Score(frame, history)
		if err != nil {
			fmt.Fprintln(os.Stderr, "detection:", err)
			return
		}
		if graph != nil {
			graph.Push(score)
		}
		if !mon.Paused() && alarm == nil && score >= mon.ThresholdOf(detector.Name()) && ts.Sub(mon.LastAlarm()) > config.AlarmCooldown {
			mon.Alarm(ts)
			// the motion has only just started: wait AlarmDelay and send the
			// GIF then, so it also covers what comes after the detection
			alarm = &pendingAlarm{at: ts, due: ts.Add(config.AlarmDelay), detector: detector.Name(), score: score}
		}
		if alarm != nil && !ts.Before(alarm.due) {
			a := alarm
			alarm = nil
			if !mon.Paused() {
				caption := fmt.Sprintf("Alarm: motion detected at %s (%s score %.4f)", a.at.Format("15:04:05"), a.detector, a.score)
				// shrink now: the frames are only valid until this handler returns,
				// and Shrink copies them
				hdFrames := gif.Shrink(everyNth(lastFrames(history, frame, mon.GIFFrames()), config.HDGIFFrameStep), config.HDGIFWidth)
				go sendAlarm(bot, gifs, hdFrames, caption)
			}
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

// sendAlarm sends the low quality GIF of the alarm with a code under it, and
// saves the HD one in gifs under that code for /hd. Without a store, or if
// the HD GIF can't be made or saved, the code is left out. hdFrames are the
// frames of the HD GIF.
func sendAlarm(bot *telegram.Bot, gifs *gif.Store, hdFrames []image.Image, caption string) {
	lowFrames := gif.Shrink(everyNth(hdFrames, config.LowGIFFrameStep), config.LowGIFWidth)
	low, err := gif.Encode(lowFrames, config.LowGIFFrameDelay, false)
	if err != nil {
		fmt.Fprintln(os.Stderr, "gif:", err)
		if err = bot.SendMessage(caption); err != nil {
			fmt.Fprintln(os.Stderr, err)
		}
		return
	}

	// the HD GIF is saved before the alarm goes out, so the code works at once
	if gifs != nil {
		if code, err := saveHD(gifs, hdFrames); err != nil {
			fmt.Fprintln(os.Stderr, "hd gif:", err)
		} else {
			caption += fmt.Sprintf("\nHD version: /hd %s", code)
		}
	}
	if err := bot.SendAnimation(low, caption); err != nil {
		fmt.Fprintln(os.Stderr, err)
	}
}

// saveHD encodes frames as the HD GIF and saves it, returning its code.
func saveHD(gifs *gif.Store, frames []image.Image) (string, error) {
	hd, err := gif.Encode(frames, config.HDGIFFrameDelay, config.HDGIFDither)
	if err != nil {
		return "", err
	}
	code, err := gif.NewCode()
	if err != nil {
		return "", err
	}
	return code, gifs.Save(code, hd)
}

// pendingAlarm is a detection whose alarm is sent once due has passed.
type pendingAlarm struct {
	at, due  time.Time
	detector string
	score    float64
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

// everyNth returns one frame out of every n, always keeping the last one.
func everyNth(frames []image.Image, n int) []image.Image {
	if n <= 1 {
		return frames
	}
	out := make([]image.Image, 0, (len(frames)+n-1)/n)
	for i := (len(frames) - 1) % n; i < len(frames); i += n {
		out = append(out, frames[i])
	}
	return out
}
