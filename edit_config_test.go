package main

import (
	"encoding/json"
	"strings"
	"testing"
	"time"

	"security_camera/lib/config"
	"security_camera/lib/detection"
	"security_camera/lib/monitor"
	"security_camera/lib/streaming"
)

func newTestSettings() (*monitor.Monitor, *streaming.Stream, *config.Live, []string) {
	mon := monitor.New("", detection.NameBackground, map[string]float64{
		detection.NameBackground: config.BackgroundThreshold,
		detection.NameDifference: config.DifferenceThreshold,
	}, config.AlarmCooldown)
	mon.SetDetectionFrames(config.DetectionFrames)
	mon.SetGIFFrames(config.GIFFrames)
	stream := streaming.New(nil, streaming.Options{History: config.StreamHistory})
	return mon, stream, config.NewLive(), []string{detection.NameBackground, detection.NameDifference}
}

func TestEditConfigRoundTrip(t *testing.T) {
	mon, stream, live, names := newTestSettings()
	before := configJSON(currentConfig(mon, stream, live))
	if err := applyConfig(before, mon, stream, live, names); err != nil {
		t.Fatal(err)
	}
	if after := configJSON(currentConfig(mon, stream, live)); after != before {
		t.Errorf("sending the JSON back changed it:\n%s\n%s", before, after)
	}
}

func TestEditConfigApplies(t *testing.T) {
	mon, stream, live, names := newTestSettings()
	err := applyConfig(`{"detector": "Difference", "difference_threshold": 0.2, "alarm_cooldown_seconds": 12.5,
		"alarm_delay_seconds": 1, "paused": true, "stack_frames": 400, "gif_frames": 60, "hd_gif_dither": false, "snap_jpeg_quality": 50}`, mon, stream, live, names)
	if err != nil {
		t.Fatal(err)
	}
	if mon.Detector() != detection.NameDifference || mon.ThresholdOf(detection.NameDifference) != 0.2 || !mon.Paused() {
		t.Errorf("monitor: detector %s threshold %v paused %v", mon.Detector(), mon.ThresholdOf(detection.NameDifference), mon.Paused())
	}
	if mon.Cooldown() != 12500*time.Millisecond || stream.History() != 400 || mon.GIFFrames() != 60 {
		t.Errorf("cooldown %v history %d gif frames %d", mon.Cooldown(), stream.History(), mon.GIFFrames())
	}
	if v := live.Get(); v.AlarmDelay != time.Second || v.HDGIFDither || v.SnapJPEGQuality != 50 || v.HDGIFWidth != config.HDGIFWidth {
		t.Errorf("live values: %+v", v)
	}
}

func TestEditConfigRejectsWithoutChanging(t *testing.T) {
	for payload, want := range map[string]string{
		`{"detector": "magic"}`:                    "unknown detector",
		`{"background_threshold": 2}`:              "background_threshold",
		`{"detection_frames": 1}`:                  "detection_frames",
		`{"stack_frames": 10}`:                     "stack_frames",
		`{"alarm_cooldown_seconds": -1}`:           "alarm_cooldown_seconds",
		`{"hd_gif_width": 0}`:                      "hd_gif_width",
		`{"snap_jpeg_quality": 101}`:               "snap_jpeg_quality",
		`{"no_such_setting": 1}`:                   "unknown field",
		`{"alarm_delay_seconds": "x"}`:             "not valid settings JSON",
		`{"paused": true} trailing`:                "not valid settings JSON",
		`not json`:                                 "not valid settings JSON",
		`{"paused": true, "snap_jpeg_quality": 0}`: "snap_jpeg_quality",
	} {
		mon, stream, live, names := newTestSettings()
		before := configJSON(currentConfig(mon, stream, live))
		err := applyConfig(payload, mon, stream, live, names)
		if err == nil || !strings.Contains(err.Error(), want) {
			t.Errorf("%s: error %v, want one with %q", payload, err, want)
		}
		if after := configJSON(currentConfig(mon, stream, live)); after != before {
			t.Errorf("%s: settings changed despite the error", payload)
		}
	}
}

func TestEditConfigJSONIsValid(t *testing.T) {
	mon, stream, live, _ := newTestSettings()
	var m map[string]any
	if err := json.Unmarshal([]byte(configJSON(currentConfig(mon, stream, live))), &m); err != nil {
		t.Fatal(err)
	}
	if m["detector"] != "background" || m["alarm_cooldown_seconds"] != 5.0 {
		t.Errorf("unexpected JSON: %v", m)
	}
}
