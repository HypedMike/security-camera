// Package alarmlog keeps track of the latest alarms: the message that told
// about each one and the GIF that is recorded after it.
//
// An alarm starts as Recording, the GIF is being made. It becomes Ready when
// the GIF is saved, or Failed if it could not be. Whoever reacts to the alarm
// message with a thumbs up asks for the GIF: Request says whether to send it
// now or, if it is still being recorded, once it is ready.
//
// The log is implemented in Rust, see rust-lib/alarmlog/src/lib.rs.
package alarmlog

/*
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

void *alarmlog_new(void);
void alarmlog_free(void *log);
uint64_t alarmlog_begin(const void *log, int64_t at, int32_t utc_offset, const uint8_t *detector, size_t detector_len, double score, const uint8_t *code, size_t code_len);
int64_t alarmlog_message_id(const void *log, uint64_t id);
void alarmlog_posted(const void *log, uint64_t id, int64_t message_id);
bool alarmlog_finish(const void *log, uint64_t id, bool failed, const uint8_t *reason, size_t reason_len);
void alarmlog_unsent(const void *log, uint64_t id);
int32_t alarmlog_request(const void *log, int64_t message_id, uint64_t *id);
uint8_t *alarmlog_report(const void *log, size_t n, size_t *len);
void alarmlog_free_text(uint8_t *ptr, size_t len);
*/
import "C"

import (
	"runtime"
	"sync"
	"time"
	"unsafe"

	_ "security_camera/lib/native"
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

// kept is how many alarms are remembered. It is KEPT in rust-lib/alarmlog,
// which forgets an alarm when the one kept alarms after it begins.
const kept = 100

// Log remembers the latest alarms. The zero value is ready to use.
type Log struct {
	once sync.Once
	rust *rustLog

	mu     sync.Mutex
	alarms map[uint64]*Alarm // the ones Rust still remembers
}

// rustLog owns the log in Rust: the finalizer is on this and not on Log, so
// a Log can be part of a bigger struct.
type rustLog struct{ p unsafe.Pointer }

// Alarm is one alarm in a Log.
type Alarm struct {
	log  *Log
	id   uint64
	at   time.Time
	code string
}

// ptr is the log in Rust, made when it is first needed.
func (l *Log) ptr() unsafe.Pointer {
	l.once.Do(func() {
		l.rust = &rustLog{p: C.alarmlog_new()}
		runtime.SetFinalizer(l.rust, func(r *rustLog) { C.alarmlog_free(r.p) })
	})
	return l.rust.p
}

// cbytes is the address of the first byte of s, or nil if there is none.
func cbytes(s string) *C.uint8_t {
	if s == "" {
		return nil
	}
	return (*C.uint8_t)(unsafe.Pointer(unsafe.StringData(s)))
}

// Begin records a new alarm detected at at, in the Recording state. code is
// what the GIF is saved under.
func (l *Log) Begin(at time.Time, detector string, score float64, code string) *Alarm {
	_, offset := at.Zone()
	l.mu.Lock()
	defer l.mu.Unlock()
	id := uint64(C.alarmlog_begin(l.ptr(), C.int64_t(at.Unix()), C.int32_t(offset),
		cbytes(detector), C.size_t(len(detector)), C.double(score), cbytes(code), C.size_t(len(code))))
	a := &Alarm{log: l, id: id, at: at, code: code}
	if l.alarms == nil {
		l.alarms = map[uint64]*Alarm{}
	}
	l.alarms[id] = a
	if id >= kept {
		delete(l.alarms, id-kept)
	}
	return a
}

// At is when the alarm went off.
func (a *Alarm) At() time.Time { return a.at }

// MessageID is the id of the alarm message, or 0 if it is not sent (yet).
func (a *Alarm) MessageID() int {
	return int(C.alarmlog_message_id(a.log.ptr(), C.uint64_t(a.id)))
}

// Code is what the GIF of the alarm is saved under.
func (a *Alarm) Code() string { return a.code }

// Posted says the message of the alarm was sent as message messageID, so a
// reaction to that message asks for the GIF.
func (a *Alarm) Posted(messageID int) {
	C.alarmlog_posted(a.log.ptr(), C.uint64_t(a.id), C.int64_t(messageID))
}

// Finish says the GIF is saved, or, if err is not nil, that it could not be
// made. It returns true if somebody has already asked for the GIF and it is
// to be sent now.
func (a *Alarm) Finish(err error) (send bool) {
	var reason string
	if err != nil {
		reason = err.Error()
	}
	return bool(C.alarmlog_finish(a.log.ptr(), C.uint64_t(a.id), C.bool(err != nil), cbytes(reason), C.size_t(len(reason))))
}

// Unsent says the GIF could not be sent after Request or Finish told to send
// it, so the next request tries again.
func (a *Alarm) Unsent() {
	C.alarmlog_unsent(a.log.ptr(), C.uint64_t(a.id))
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
	var id C.uint64_t
	action := Action(C.alarmlog_request(l.ptr(), C.int64_t(messageID), &id))
	if action == Unknown {
		return nil, Unknown
	}
	l.mu.Lock()
	a := l.alarms[uint64(id)]
	l.mu.Unlock()
	if a == nil {
		return nil, Unknown
	}
	return a, action
}

// Report describes the latest n alarms for a Telegram message.
func (l *Log) Report(n int) string {
	var size C.size_t
	p := C.alarmlog_report(l.ptr(), C.size_t(max(n, 0)), &size)
	defer C.alarmlog_free_text(p, size)
	return C.GoStringN((*C.char)(unsafe.Pointer(p)), C.int(size))
}
