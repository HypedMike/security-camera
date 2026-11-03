// Package syswatch warns on Telegram when the machine is in trouble: the disk
// is almost full, the CPU or its temperature stay high, or the connection to
// Telegram was lost.
package syswatch

import (
	"context"
	"fmt"
	"time"
)

// Limits says when a reading is a problem.
type Limits struct {
	// DiskPercent is the disk usage (0 to 100) from which the disk is almost full.
	DiskPercent float64
	// CPUPercent is the CPU usage (0 to 100) that is a problem when it lasts
	// for CPUChecks checks in a row: short peaks are normal.
	CPUPercent float64
	CPUChecks  int
	// TempC is the temperature in °C from which the machine is too hot.
	TempC float64
	// OfflineChecks is how many failed connection checks in a row mean the
	// connection is lost: a single failure is often just a hiccup.
	OfflineChecks int
}

// hysteresis is how far below its limit a reading must fall before the
// problem is over, so a value hovering around the limit does not send a
// message at every check.
const hysteresis = 5

// Reading is what the machine looks like at one check. A value that could
// not be read is left out with its Has flag false.
type Reading struct {
	Disk, CPU, Temp          float64
	HasDisk, HasCPU, HasTemp bool
	// Online says whether Telegram could be reached.
	Online bool
}

// Watcher turns readings into messages. It remembers which problems are
// ongoing, so each one is reported when it starts and when it ends, not at
// every check. It is not safe for concurrent use.
type Watcher struct {
	limits Limits

	disk, cpu, temp bool // problem ongoing
	cpuHigh         int  // checks in a row with the CPU over its limit

	failures     int       // failed connection checks in a row
	firstFailure time.Time // when the current run of failures began
	offline      bool      // the connection is considered lost
}

// New returns a Watcher with the given limits.
func New(l Limits) *Watcher {
	if l.CPUChecks < 1 {
		l.CPUChecks = 1
	}
	if l.OfflineChecks < 1 {
		l.OfflineChecks = 1
	}
	return &Watcher{limits: l}
}

// Update takes the reading made at now and returns the messages to send.
// A lost connection is only reported when it comes back, because there is no
// way to tell Telegram before.
func (w *Watcher) Update(now time.Time, r Reading) []string {
	var out []string

	if r.HasDisk {
		if msg := step(&w.disk, r.Disk, w.limits.DiskPercent,
			"Disk almost full: %.0f%% used (alert from %.0f%%)", "Disk back to normal: %.0f%% used"); msg != "" {
			out = append(out, msg)
		}
	}

	if r.HasCPU {
		if r.CPU >= w.limits.CPUPercent {
			w.cpuHigh++
		} else {
			w.cpuHigh = 0
		}
		high := w.cpu
		if !w.cpu && w.cpuHigh >= w.limits.CPUChecks {
			high = true
			out = append(out, fmt.Sprintf("CPU busy: %.0f%% for %d checks in a row (alert from %.0f%%)", r.CPU, w.cpuHigh, w.limits.CPUPercent))
		} else if w.cpu && r.CPU < w.limits.CPUPercent-hysteresis {
			high = false
			out = append(out, fmt.Sprintf("CPU back to normal: %.0f%%", r.CPU))
		}
		w.cpu = high
	}

	if r.HasTemp {
		if msg := step(&w.temp, r.Temp, w.limits.TempC,
			"Temperature high: %.0f°C (alert from %.0f°C)", "Temperature back to normal: %.0f°C"); msg != "" {
			out = append(out, msg)
		}
	}

	if r.Online {
		if w.offline {
			out = append(out, fmt.Sprintf("Connection back after %s (lost at %s)",
				now.Sub(w.firstFailure).Round(time.Second), w.firstFailure.Format("15:04")))
		}
		w.failures, w.offline = 0, false
	} else {
		if w.failures == 0 {
			w.firstFailure = now
		}
		w.failures++
		if w.failures >= w.limits.OfflineChecks {
			w.offline = true
		}
	}
	return out
}

// step updates *on, whether a problem is ongoing, with a reading of v against
// its limit and returns the message for a problem that starts or ends, "" if
// nothing changed. start is formatted with v and limit, end with v.
func step(on *bool, v, limit float64, start, end string) string {
	switch {
	case !*on && v >= limit:
		*on = true
		return fmt.Sprintf(start, v, limit)
	case *on && v < limit-hysteresis:
		*on = false
		return fmt.Sprintf(end, v)
	}
	return ""
}

// Run checks the machine every interval until ctx is done and sends what
// Update returns. A message that cannot be sent (the connection is down) is
// kept and tried again at the next check, in order.
func Run(ctx context.Context, interval time.Duration, l Limits, check func(context.Context) Reading, send func(string) error) {
	w := New(l)
	var pending []string
	t := time.NewTicker(interval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case now := <-t.C:
			r := check(ctx)
			pending = append(pending, w.Update(now, r)...)
			for len(pending) > 0 && r.Online {
				if err := send(pending[0]); err != nil {
					break
				}
				pending = pending[1:]
			}
		}
	}
}
