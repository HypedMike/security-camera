package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"strings"
	"time"

	"security_camera/lib/config"
	"security_camera/lib/detection"
	"security_camera/lib/monitor"
	"security_camera/lib/streaming"
)

// editableConfig is the JSON /edit_config sends and accepts. Times are in
// seconds; the GIF delays are in hundredths of a second, as in the config
// package.
type editableConfig struct {
	Detector            string  `json:"detector"`
	BackgroundThreshold float64 `json:"background_threshold"`
	DifferenceThreshold float64 `json:"difference_threshold"`
	// BackgroundRustThreshold only matters in builds with -tags rust
	BackgroundRustThreshold float64 `json:"background_rust_threshold"`
	Paused                  bool    `json:"paused"`

	StackFrames     int `json:"stack_frames"`
	DetectionFrames int `json:"detection_frames"`
	GIFFrames       int `json:"gif_frames"`

	AlarmCooldownSeconds float64 `json:"alarm_cooldown_seconds"`
	AlarmDelaySeconds    float64 `json:"alarm_delay_seconds"`

	HDGIFFrameStep  int  `json:"hd_gif_frame_step"`
	HDGIFFrameDelay int  `json:"hd_gif_frame_delay"`
	HDGIFWidth      int  `json:"hd_gif_width"`
	HDGIFDither     bool `json:"hd_gif_dither"`

	LowGIFFrameStep  int `json:"low_gif_frame_step"`
	LowGIFFrameDelay int `json:"low_gif_frame_delay"`
	LowGIFWidth      int `json:"low_gif_width"`

	SnapJPEGQuality int `json:"snap_jpeg_quality"`
}

// currentConfig returns the settings the program is running with.
func currentConfig(mon *monitor.Monitor, stream *streaming.Stream, live *config.Live) editableConfig {
	v := live.Get()
	return editableConfig{
		Detector:                mon.Detector(),
		BackgroundThreshold:     mon.ThresholdOf(detection.NameBackground),
		DifferenceThreshold:     mon.ThresholdOf(detection.NameDifference),
		BackgroundRustThreshold: mon.ThresholdOf(detection.NameBackgroundRust),
		Paused:                  mon.Paused(),

		StackFrames:     stream.History(),
		DetectionFrames: mon.DetectionFrames(),
		GIFFrames:       mon.GIFFrames(),

		AlarmCooldownSeconds: mon.Cooldown().Seconds(),
		AlarmDelaySeconds:    v.AlarmDelay.Seconds(),

		HDGIFFrameStep:  v.HDGIFFrameStep,
		HDGIFFrameDelay: v.HDGIFFrameDelay,
		HDGIFWidth:      v.HDGIFWidth,
		HDGIFDither:     v.HDGIFDither,

		LowGIFFrameStep:  v.LowGIFFrameStep,
		LowGIFFrameDelay: v.LowGIFFrameDelay,
		LowGIFWidth:      v.LowGIFWidth,

		SnapJPEGQuality: v.SnapJPEGQuality,
	}
}

// validate returns why c can't be used, or nil. detectors are the names of
// the known motion detectors.
func (c editableConfig) validate(detectors []string) error {
	var known bool
	for _, d := range detectors {
		known = known || d == c.Detector
	}
	if !known {
		return fmt.Errorf("detector: unknown detector %q, use one of: %s", c.Detector, strings.Join(detectors, ", "))
	}
	for _, f := range []struct {
		name string
		v    float64
	}{{"background_threshold", c.BackgroundThreshold}, {"difference_threshold", c.DifferenceThreshold}, {"background_rust_threshold", c.BackgroundRustThreshold}} {
		if math.IsNaN(f.v) || f.v < 0 || f.v > 1 {
			return fmt.Errorf("%s: must be between 0 and 1", f.name)
		}
	}
	if c.DetectionFrames < 2 {
		return errors.New("detection_frames: must be at least 2")
	}
	if c.GIFFrames < 2 {
		return errors.New("gif_frames: must be at least 2")
	}
	if c.StackFrames < max(c.DetectionFrames, c.GIFFrames) {
		return fmt.Errorf("stack_frames: must be at least %d, the larger of detection_frames and gif_frames", max(c.DetectionFrames, c.GIFFrames))
	}
	for _, f := range []struct {
		name string
		v    float64
	}{{"alarm_cooldown_seconds", c.AlarmCooldownSeconds}, {"alarm_delay_seconds", c.AlarmDelaySeconds}} {
		if math.IsNaN(f.v) || math.IsInf(f.v, 0) || f.v < 0 {
			return fmt.Errorf("%s: must be 0 or more", f.name)
		}
	}
	for _, f := range []struct {
		name string
		v    int
	}{
		{"hd_gif_frame_step", c.HDGIFFrameStep}, {"hd_gif_frame_delay", c.HDGIFFrameDelay}, {"hd_gif_width", c.HDGIFWidth},
		{"low_gif_frame_step", c.LowGIFFrameStep}, {"low_gif_frame_delay", c.LowGIFFrameDelay}, {"low_gif_width", c.LowGIFWidth},
	} {
		if f.v < 1 {
			return fmt.Errorf("%s: must be at least 1", f.name)
		}
	}
	if c.SnapJPEGQuality < 1 || c.SnapJPEGQuality > 100 {
		return errors.New("snap_jpeg_quality: must be between 1 and 100")
	}
	return nil
}

// applyConfig changes the settings to the JSON in payload. Keys it leaves out
// keep their value. Nothing is changed unless all of it is valid.
func applyConfig(payload string, mon *monitor.Monitor, stream *streaming.Stream, live *config.Live, detectors []string) error {
	c := currentConfig(mon, stream, live)
	dec := json.NewDecoder(strings.NewReader(payload))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&c); err != nil {
		return fmt.Errorf("not valid settings JSON: %w", err)
	}
	if _, err := dec.Token(); !errors.Is(err, io.EOF) {
		return errors.New("not valid settings JSON: unexpected text after the JSON")
	}
	c.Detector = strings.ToLower(c.Detector)
	if err := c.validate(detectors); err != nil {
		return err
	}

	mon.SetThresholdOf(detection.NameBackground, c.BackgroundThreshold)
	mon.SetThresholdOf(detection.NameDifference, c.DifferenceThreshold)
	mon.SetThresholdOf(detection.NameBackgroundRust, c.BackgroundRustThreshold)
	mon.SetDetector(c.Detector)
	mon.SetPaused(c.Paused)
	mon.SetCooldown(secondsToDuration(c.AlarmCooldownSeconds))
	stream.SetHistory(c.StackFrames)
	mon.SetDetectionFrames(c.DetectionFrames)
	mon.SetGIFFrames(c.GIFFrames)
	live.Set(config.Values{
		AlarmDelay:       secondsToDuration(c.AlarmDelaySeconds),
		HDGIFFrameStep:   c.HDGIFFrameStep,
		HDGIFFrameDelay:  c.HDGIFFrameDelay,
		HDGIFWidth:       c.HDGIFWidth,
		HDGIFDither:      c.HDGIFDither,
		LowGIFFrameStep:  c.LowGIFFrameStep,
		LowGIFFrameDelay: c.LowGIFFrameDelay,
		LowGIFWidth:      c.LowGIFWidth,
		SnapJPEGQuality:  c.SnapJPEGQuality,
	})
	return nil
}

func secondsToDuration(s float64) time.Duration {
	return time.Duration(s * float64(time.Second))
}

// configJSON returns c as indented JSON.
func configJSON(c editableConfig) string {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetIndent("", "  ")
	_ = enc.Encode(c)
	return strings.TrimSpace(buf.String())
}
