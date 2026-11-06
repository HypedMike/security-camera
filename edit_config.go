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

// The longest alarm GIF and the most frames a second it can have: the frames
// of a GIF wait in memory until it is encoded.
const (
	maxAlarmGIFSeconds = 30
	maxAlarmGIFFPS     = 10
)

// editableConfig is the JSON /edit_config sends and accepts. Times are in
// seconds.
type editableConfig struct {
	Detector            string  `json:"detector"`
	BackgroundThreshold float64 `json:"background_threshold"`
	DifferenceThreshold float64 `json:"difference_threshold"`
	// BackgroundRustThreshold only matters in builds with -tags rust
	BackgroundRustThreshold float64 `json:"background_rust_threshold"`
	Paused                  bool    `json:"paused"`

	StackFrames     int `json:"stack_frames"`
	DetectionFrames int `json:"detection_frames"`

	AlarmCooldownSeconds float64 `json:"alarm_cooldown_seconds"`

	AlarmGIFSeconds float64 `json:"alarm_gif_seconds"`
	AlarmGIFFPS     int     `json:"alarm_gif_fps"`
	HDGIFWidth      int     `json:"hd_gif_width"`
	HDGIFDither     bool    `json:"hd_gif_dither"`

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

		AlarmCooldownSeconds: mon.Cooldown().Seconds(),

		AlarmGIFSeconds: v.AlarmGIFSpan.Seconds(),
		AlarmGIFFPS:     v.AlarmGIFFPS,
		HDGIFWidth:      v.HDGIFWidth,
		HDGIFDither:     v.HDGIFDither,

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
	if c.StackFrames < c.DetectionFrames {
		return fmt.Errorf("stack_frames: must be at least %d, detection_frames", c.DetectionFrames)
	}
	for _, f := range []struct {
		name string
		v    float64
	}{{"alarm_cooldown_seconds", c.AlarmCooldownSeconds}} {
		if math.IsNaN(f.v) || math.IsInf(f.v, 0) || f.v < 0 {
			return fmt.Errorf("%s: must be 0 or more", f.name)
		}
	}
	if math.IsNaN(c.AlarmGIFSeconds) || c.AlarmGIFSeconds < 0.5 || c.AlarmGIFSeconds > maxAlarmGIFSeconds {
		return fmt.Errorf("alarm_gif_seconds: must be between 0.5 and %d", maxAlarmGIFSeconds)
	}
	for _, f := range []struct {
		name string
		v    int
	}{
		{"alarm_gif_fps", c.AlarmGIFFPS}, {"hd_gif_width", c.HDGIFWidth},
	} {
		if f.v < 1 {
			return fmt.Errorf("%s: must be at least 1", f.name)
		}
	}
	if c.AlarmGIFFPS > maxAlarmGIFFPS {
		return fmt.Errorf("alarm_gif_fps: must be at most %d", maxAlarmGIFFPS)
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
	live.Set(config.Values{
		AlarmGIFSpan:    secondsToDuration(c.AlarmGIFSeconds),
		AlarmGIFFPS:     c.AlarmGIFFPS,
		HDGIFWidth:      c.HDGIFWidth,
		HDGIFDither:     c.HDGIFDither,
		SnapJPEGQuality: c.SnapJPEGQuality,
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
