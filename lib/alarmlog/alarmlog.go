// Package alarmlog keeps track of the latest alarms: the message that told
// about each one and the GIF that is recorded after it.
//
// An alarm starts as Recording, the GIF is being made. It becomes Ready when
// the GIF is saved, or Failed if it could not be. Whoever reacts to the alarm
// message with a thumbs up asks for the GIF: Request says whether to send it
// now or, if it is still being recorded, once it is ready.
package alarmlog

import (
	"fmt"
	"strings"
	"sync"
	"time"
)

// Status is where the GIF of an alarm is.
type Status int

const (
	// Recording: the frames are still being captured or encoded.
	Recording Status = iota
	// Ready: the GIF is saved and can be sent.
	Ready
	// Failed: the GIF could not be made or saved.
	Failed
)

// kept is how many alarms are remembered.
const kept = 100

// Log remembers the latest alarms. The zero value is ready to use.
type Log struct {
	mu        sync.Mutex
	alarms    []*Alarm // oldest first, at most kept
	byMessage map[int]*Alarm
}

// Alarm is one alarm in a Log.
type Alarm struct {
	log      *Log
	at       time.Time
	detector string
	score    float64
	code     string

	// guarded by log.mu
	messageID int
	status    Status
	reason    string
	wanted    bool // somebody asked for the GIF before it was ready
	claimed   bool // the GIF is being sent, or was
}

// Begin records a new alarm detected at at, in the Recording state. code is
// what the GIF is saved under.
func (l *Log) Begin(at time.Time, detector string, score float64, code string) *Alarm {
	a := &Alarm{log: l, at: at, detector: detector, score: score, code: code}
	l.mu.Lock()
	defer l.mu.Unlock()
	l.alarms = append(l.alarms, a)
	if len(l.alarms) > kept {
		for _, old := range l.alarms[:len(l.alarms)-kept] {
			delete(l.byMessage, old.messageID)
		}
		l.alarms = l.alarms[len(l.alarms)-kept:]
	}
	return a
}

// At is when the alarm went off.
func (a *Alarm) At() time.Time { return a.at }

// MessageID is the id of the alarm message, or 0 if it is not sent (yet).
func (a *Alarm) MessageID() int {
	a.log.mu.Lock()
	defer a.log.mu.Unlock()
	return a.messageID
}

// Code is what the GIF of the alarm is saved under.
func (a *Alarm) Code() string { return a.code }

// Posted says the message of the alarm was sent as message messageID, so a
// reaction to that message asks for the GIF.
func (a *Alarm) Posted(messageID int) {
	l := a.log
	l.mu.Lock()
	defer l.mu.Unlock()
	if messageID == 0 {
		return
	}
	a.messageID = messageID
	if l.byMessage == nil {
		l.byMessage = map[int]*Alarm{}
	}
	l.byMessage[messageID] = a
}

// Finish says the GIF is saved, or, if err is not nil, that it could not be
// made. It returns true if somebody has already asked for the GIF and it is
// to be sent now.
func (a *Alarm) Finish(err error) (send bool) {
	l := a.log
	l.mu.Lock()
	defer l.mu.Unlock()
	if a.status != Recording {
		return false
	}
	if err != nil {
		a.status, a.reason = Failed, err.Error()
		return false
	}
	a.status = Ready
	if a.wanted && !a.claimed {
		a.claimed = true
		return true
	}
	return false
}

// Unsent says the GIF could not be sent after Request or Finish told to send
// it, so the next request tries again.
func (a *Alarm) Unsent() {
	a.log.mu.Lock()
	defer a.log.mu.Unlock()
	a.claimed = false
}

// Action is what to do about a request for a GIF.
type Action int

const (
	// Unknown: no alarm has that message (it is not an alarm, or too old).
	Unknown Action = iota
	// Send the GIF now.
	Send
	// Later: the GIF is still being recorded, Finish will say when to send it.
	Later
	// Nothing: the GIF was already sent, or is being.
	Nothing
	// NoGIF: the GIF could not be made.
	NoGIF
)

// Request is called when somebody reacts to the message messageID. It returns
// the alarm that message is of and what to do.
func (l *Log) Request(messageID int) (*Alarm, Action) {
	l.mu.Lock()
	defer l.mu.Unlock()
	a := l.byMessage[messageID]
	if a == nil {
		return nil, Unknown
	}
	switch a.status {
	case Failed:
		return a, NoGIF
	case Recording:
		if a.wanted {
			return a, Nothing
		}
		a.wanted = true
		return a, Later
	}
	if a.claimed {
		return a, Nothing
	}
	a.claimed = true
	return a, Send
}

// Info is the state of an alarm.
type Info struct {
	At       time.Time
	Detector string
	Score    float64
	Code     string
	Status   Status
	// Sent is whether the GIF has been asked for and sent.
	Sent bool
	// Reason is why a Failed alarm has no GIF.
	Reason string
}

// Recent returns the state of the latest n alarms, newest first.
func (l *Log) Recent(n int) []Info {
	l.mu.Lock()
	defer l.mu.Unlock()
	var out []Info
	for i := len(l.alarms) - 1; i >= 0 && len(out) < n; i-- {
		a := l.alarms[i]
		out = append(out, Info{At: a.at, Detector: a.detector, Score: a.score, Code: a.code, Status: a.status,
			Sent: a.claimed && a.status == Ready, Reason: a.reason})
	}
	return out
}

// Report describes the latest n alarms for a Telegram message.
func (l *Log) Report(n int) string {
	alarms := l.Recent(n)
	if len(alarms) == 0 {
		return "No alarms yet."
	}
	var b strings.Builder
	fmt.Fprintf(&b, "Latest %d alarms, newest first:", len(alarms))
	for _, a := range alarms {
		fmt.Fprintf(&b, "\n%s %s %.4f: ", a.At.Format("15:04:05"), a.Detector, a.Score)
		switch {
		case a.Status == Recording:
			b.WriteString("recording the GIF")
		case a.Status == Failed:
			fmt.Fprintf(&b, "no GIF (%s)", a.Reason)
		case a.Sent:
			fmt.Fprintf(&b, "GIF sent (%s)", a.Code)
		default:
			fmt.Fprintf(&b, "GIF ready, react with 👍 or /hd %s", a.Code)
		}
	}
	return b.String()
}
