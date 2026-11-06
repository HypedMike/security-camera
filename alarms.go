package main

import (
	"errors"
	"fmt"
	"image"
	"os"
	"time"

	"security_camera/lib/alarmlog"
	"security_camera/lib/config"
	"security_camera/lib/gif"
	"security_camera/lib/recorder"
	"security_camera/lib/telegram"
)

// thumbsUp is the reaction that asks for the GIF of an alarm.
const thumbsUp = "👍"

// startAlarm tells about the alarm at once and, in the background, records
// the GIF of it: the frame that set it off and the ones of the next
// v.AlarmGIFSpan, at v.AlarmGIFFPS a second. The GIF is saved in gifs under
// the code in the message, and goes out when somebody reacts to the message
// with a thumbs up. frame is only valid until the caller returns.
func startAlarm(bot *telegram.Bot, log *alarmlog.Log, rec *recorder.Recorder, gifs *gif.Store, at time.Time, frame image.Image, detector string, score float64, v config.Values) {
	// without a code the alarm still goes out, it just has no GIF
	code, codeErr := gif.NewCode()
	a := log.Begin(at, detector, score, code)

	text := fmt.Sprintf("Alarm: motion detected at %s (%s score %.4f)", at.Format("15:04:05"), detector, score)
	if codeErr == nil && gifs != nil {
		text += fmt.Sprintf("\nReact with %s to get the GIF (or send /hd %s)", thumbsUp, code)
	}
	// the message must not wait for the camera loop, nor the recording for the message
	go func() {
		id, err := bot.Send(text)
		if err != nil {
			fmt.Fprintln(os.Stderr, "alarm:", err)
			return
		}
		a.Posted(id)
	}()

	interval := time.Second / time.Duration(v.AlarmGIFFPS)
	// the delay is in hundredths of a second
	delay := max(1, int(interval.Milliseconds()/10))
	rec.Start(at, frame, v.AlarmGIFSpan, interval, func(frames []image.Image) {
		err := codeErr
		if err == nil && gifs == nil {
			err = errors.New("GIFs are not being saved")
		}
		if err == nil {
			err = saveGIF(gifs, code, frames, delay, v)
		}
		if err != nil {
			fmt.Fprintln(os.Stderr, "alarm gif:", err)
		}
		// somebody may have asked for the GIF while it was being made
		if a.Finish(err) {
			sendGIF(bot, gifs, a)
		}
	})
}

// saveGIF encodes frames as a GIF, delay hundredths of a second apart, and
// saves it under code.
func saveGIF(gifs *gif.Store, code string, frames []image.Image, delay int, v config.Values) error {
	data, err := gif.Encode(frames, delay, v.HDGIFDither)
	if err != nil {
		return err
	}
	return gifs.Save(code, data)
}

// sendGIF sends the saved GIF of the alarm as a reply to its message. If that
// fails the next request for it tries again.
func sendGIF(bot *telegram.Bot, gifs *gif.Store, a *alarmlog.Alarm) {
	data, err := gifs.Get(a.Code())
	if err == nil {
		err = bot.SendAnimationReply(data, "GIF of the alarm at "+a.At().Format("15:04:05"), a.MessageID())
	}
	if err == nil {
		return
	}
	a.Unsent()
	fmt.Fprintln(os.Stderr, "alarm gif:", err)
	msg := "Could not send the GIF."
	if errors.Is(err, gif.ErrNotFound) {
		msg = "The GIF is gone: old ones are deleted to keep the storage under the limit."
	}
	if err := bot.SendMessage(msg); err != nil {
		fmt.Fprintln(os.Stderr, "alarm gif:", err)
	}
}

// onReaction sends the GIF of an alarm when its message gets a thumbs up.
func onReaction(bot *telegram.Bot, log *alarmlog.Log, gifs *gif.Store, r telegram.Reaction) {
	if r.Emoji != thumbsUp {
		return
	}
	a, action := log.Request(r.MessageID)
	switch action {
	case alarmlog.Send:
		// uploading takes a while, don't hold up the other updates
		go sendGIF(bot, gifs, a)
	case alarmlog.NoGIF:
		go func() {
			if err := bot.SendMessage("There is no GIF of that alarm: it could not be made."); err != nil {
				fmt.Fprintln(os.Stderr, "alarm gif:", err)
			}
		}()
	}
}
