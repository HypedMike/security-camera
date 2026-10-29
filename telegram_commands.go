package main

import (
	"context"
	"fmt"
	"os"
	"sync/atomic"

	"security_camera/lib/config"
	"security_camera/lib/monitor"
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
//	/update  pulls, rebuilds and restarts the program
//
// cancel stops the stream, which is how /update makes main restart.
func initTelegramCommands(ctx context.Context, cancel context.CancelFunc, bot *telegram.Bot, mon *monitor.Monitor, state *commandState) {
	reply := func(command, text string) {
		if err := bot.SendMessage(text); err != nil {
			fmt.Fprintf(os.Stderr, "%s: %v\n", command, err)
		}
	}

	var updating atomic.Bool

	commands := map[string]func(){
		"snap": func() { state.snapRequested.Store(true) },

		"info": func() {
			// the report takes a moment to measure, don't hold up the other messages
			go reply("info", sysinfo.Report(".")+"\n\n"+mon.Report())
		},

		"update": func() {
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
			run()
		}
	})
}
