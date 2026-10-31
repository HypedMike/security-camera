package main

import (
	"context"
	"errors"
	"fmt"
	"maps"
	"os"
	"slices"
	"strings"
	"sync/atomic"

	"security_camera/lib/alarmqueue"
	"security_camera/lib/config"
	"security_camera/lib/detection"
	"security_camera/lib/gif"
	"security_camera/lib/monitor"
	"security_camera/lib/streaming"
	"security_camera/lib/sysinfo"
	"security_camera/lib/telegram"
	"security_camera/lib/update"
)

// queueShown is how many alarms /queue lists.
const queueShown = 10

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
//	/edit_config [json]  sends all the settings as JSON, or changes them to the JSON that follows
//	/queue   lists the latest alarms: sent (and when), waiting or still being processed
//	/hd <code>   sends the HD version of the alarm GIF with that code
//	/update  pulls, rebuilds and restarts the program
//
// cancel stops the stream, which is how /update makes main restart.
func initTelegramCommands(ctx context.Context, cancel context.CancelFunc, bot *telegram.Bot, mon *monitor.Monitor, stream *streaming.Stream, detectors map[string]detection.Detector, gifs *gif.Store, live *config.Live, alarms *alarmqueue.Queue, state *commandState) {
	reply := func(command, text string) {
		if err := bot.SendMessage(text); err != nil {
			fmt.Fprintf(os.Stderr, "%s: %v\n", command, err)
		}
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

		"edit_config": func(m telegram.Message) {
			payload := m.Payload()
			if payload == "" {
				go reply("edit_config", configJSON(currentConfig(mon, stream, live)))
				return
			}
			if err := applyConfig(payload, mon, stream, live, slices.Sorted(maps.Keys(detectors))); err != nil {
				go reply("edit_config", "Not changed: "+err.Error())
				return
			}
			go reply("edit_config", "Settings changed (until the next restart):\n"+configJSON(currentConfig(mon, stream, live)))
		},

		"queue": func(telegram.Message) {
			go reply("queue", alarms.Report(queueShown))
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
				if errors.Is(err, gif.ErrNotFound) {
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
