package main

import (
	"context"
	"errors"
	"fmt"
	"maps"
	"math"
	"os"
	"slices"
	"strconv"
	"strings"
	"sync/atomic"

	"security_camera/lib/config"
	"security_camera/lib/detection"
	"security_camera/lib/gifstore"
	"security_camera/lib/monitor"
	"security_camera/lib/streaming"
	"security_camera/lib/sysinfo"
	"security_camera/lib/telegram"
	"security_camera/lib/update"
)

// commandState is what the Telegram commands hand over to the main loop.
type commandState struct {
	// snapRequested is set by /snap: the stream handler sends the next frame.
	snapRequested atomic.Bool
	// restart is set by /update once the new build is ready: main restarts
	// when the stream has stopped.
	restart atomic.Bool
}

// initTelegramCommands registers every command the bot understands:
//
//	/snap    sends the next camera frame as a photo
//	/info    reports the state of the machine and of the program
//	/pause   stops sending alarms
//	/resume  starts sending alarms again
//	/threshold [value]  shows or changes the alarm threshold of the detector in use
//	/detector [name]    shows or changes the motion detector
//	/stack [n]   shows or changes how many frames are kept (default 300)
//	/detect [t]  shows or changes how many of the latest frames the difference detector uses (default 30)
//	/gif [m]     shows or changes how many of the latest frames an alarm GIF covers (default 150, one in 3 is kept)
//	/hd <code>   sends the HD version of the alarm GIF with that code
//	/update  pulls, rebuilds and restarts the program
//
// cancel stops the stream, which is how /update makes main restart.
func initTelegramCommands(ctx context.Context, cancel context.CancelFunc, bot *telegram.Bot, mon *monitor.Monitor, stream *streaming.Stream, detectors map[string]detection.Detector, gifs *gifstore.Store, state *commandState) {
	reply := func(command, text string) {
		if err := bot.SendMessage(text); err != nil {
			fmt.Fprintf(os.Stderr, "%s: %v\n", command, err)
		}
	}

	// setting shows an integer setting, or changes it to the value in the
	// message; apply returns why the value is refused, or "" once it is set.
	setting := func(m telegram.Message, command, what string, get func() int, apply func(int) string) {
		args := m.Args()
		if len(args) == 0 {
			go reply(command, fmt.Sprintf("%s: %d. Send /%s <number> to change it.", what, get(), command))
			return
		}
		v, err := strconv.Atoi(args[0])
		if err != nil {
			go reply(command, fmt.Sprintf("Usage: /%s <whole number>", command))
			return
		}
		if why := apply(v); why != "" {
			go reply(command, fmt.Sprintf("Not changed: %s", why))
			return
		}
		go reply(command, fmt.Sprintf("%s set to %d (until the next restart).", what, v))
	}

	var updating atomic.Bool

	commands := map[string]func(telegram.Message){
		"snap": func(telegram.Message) { state.snapRequested.Store(true) },

		"info": func(telegram.Message) {
			// the report takes a moment to measure, don't hold up the other messages
			go reply("info", sysinfo.Report(".")+"\n\n"+mon.Report())
		},

		"pause": func(telegram.Message) {
			mon.SetPaused(true)
			go reply("pause", "Alarms paused. Send /resume to turn them back on.")
		},

		"resume": func(telegram.Message) {
			mon.SetPaused(false)
			go reply("resume", "Alarms resumed.")
		},

		"threshold": func(m telegram.Message) {
			args := m.Args()
			if len(args) == 0 {
				go reply("threshold", fmt.Sprintf("Alarm threshold of the %s detector is %g. Send /threshold <value> to change it.", mon.Detector(), mon.Threshold()))
				return
			}
			t, err := strconv.ParseFloat(args[0], 64)
			if err != nil || math.IsNaN(t) || t < 0 || t > 1 {
				go reply("threshold", fmt.Sprintf("Usage: /threshold <value between 0 and 1>, e.g. /threshold %g", mon.Threshold()))
				return
			}
			mon.SetThreshold(t)
			go reply("threshold", fmt.Sprintf("Alarm threshold of the %s detector set to %g (until the next restart).", mon.Detector(), t))
		},

		"detector": func(m telegram.Message) {
			names := strings.Join(slices.Sorted(maps.Keys(detectors)), ", ")
			args := m.Args()
			if len(args) == 0 {
				go reply("detector", fmt.Sprintf("Motion detector is %s (threshold %g). Available: %s. Send /detector <name> to change it.", mon.Detector(), mon.Threshold(), names))
				return
			}
			name := strings.ToLower(args[0])
			if _, ok := detectors[name]; !ok {
				go reply("detector", fmt.Sprintf("Unknown detector %q. Available: %s.", args[0], names))
				return
			}
			mon.SetDetector(name)
			go reply("detector", fmt.Sprintf("Motion detector set to %s, threshold %g (until the next restart).", name, mon.Threshold()))
		},

		"stack": func(m telegram.Message) {
			// n must hold the frames t and m ask for, one of which is the current frame
			setting(m, "stack", "frames kept", stream.History, func(n int) string {
				if n < max(mon.DetectionFrames(), mon.GIFFrames()) {
					return fmt.Sprintf("Must be at least %d, the larger of /detect and /gif.", max(mon.DetectionFrames(), mon.GIFFrames()))
				}
				stream.SetHistory(n)
				return ""
			})
		},

		"detect": func(m telegram.Message) {
			setting(m, "detect", "frames used by the difference detector", mon.DetectionFrames, func(t int) string {
				if t < 2 {
					return "Must be at least 2."
				}
				if t > stream.History() {
					return fmt.Sprintf("Must be at most %d, the /stack size.", stream.History())
				}
				mon.SetDetectionFrames(t)
				return ""
			})
		},

		"gif": func(m telegram.Message) {
			setting(m, "gif", "frames covered by the alarm GIF", mon.GIFFrames, func(n int) string {
				if n < 2 {
					return "Must be at least 2."
				}
				if n > stream.History() {
					return fmt.Sprintf("Must be at most %d, the /stack size.", stream.History())
				}
				mon.SetGIFFrames(n)
				return ""
			})
		},

		"hd": func(m telegram.Message) {
			args := m.Args()
			if len(args) == 0 {
				go reply("hd", "Usage: /hd <code>, the code is under the alarm GIF.")
				return
			}
			if gifs == nil {
				go reply("hd", "HD GIFs are not being saved.")
				return
			}
			// reading and uploading the GIF takes a while, don't hold up the other messages
			go func() {
				data, err := gifs.Get(args[0])
				if errors.Is(err, gifstore.ErrNotFound) {
					reply("hd", fmt.Sprintf("No HD GIF with code %q. Old ones are deleted to keep the storage under the limit.", args[0]))
					return
				}
				if err == nil {
					err = bot.SendAnimation(data, "HD version of "+strings.ToLower(args[0]))
				}
				if err != nil {
					fmt.Fprintf(os.Stderr, "hd: %v\n", err)
					reply("hd", "Could not send the HD GIF.")
				}
			}()
		},

		"update": func(telegram.Message) {
			if !updating.CompareAndSwap(false, true) {
				return
			}
			// the build takes a while, don't hold up the other messages
			go func() {
				reply("update", "Updating...")
				out, err := update.Build(ctx, config.UpdateScript, config.UpdateTimeout)
				if err != nil {
					updating.Store(false)
					reply("update", "Update failed, still running the old version:\n"+update.Tail(out+"\n"+err.Error(), 3000))
					return
				}
				reply("update", update.Tail(out, 3000)+"\nRestarting...")
				state.restart.Store(true)
				cancel()
			}()
		},
	}

	bot.AddListener(func(m telegram.Message) {
		if run, ok := commands[m.Command()]; ok {
			run(m)
		}
	})
}
