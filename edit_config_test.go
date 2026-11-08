package main

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
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
		"alarm_gif_seconds": 8, "alarm_gif_fps": 4, "paused": true, "stack_frames": 400, "hd_gif_dither": false, "snap_jpeg_quality": 50}`, mon, stream, live, names)
	if err != nil {
		t.Fatal(err)
	}
	if mon.Detector() != detection.NameDifference || mon.ThresholdOf(detection.NameDifference) != 0.2 || !mon.Paused() {
		t.Errorf("monitor: detector %s threshold %v paused %v", mon.Detector(), mon.ThresholdOf(detection.NameDifference), mon.Paused())
	}
	if mon.Cooldown() != 12500*time.Millisecond || stream.History() != 400 {
		t.Errorf("cooldown %v history %d", mon.Cooldown(), stream.History())
	}
	if v := live.Get(); v.AlarmGIFSpan != 8*time.Second || v.AlarmGIFFPS != 4 || v.HDGIFDither || v.SnapJPEGQuality != 50 || v.HDGIFWidth != config.HDGIFWidth {
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
		`{"alarm_gif_seconds": "x"}`:               "not valid settings JSON",
		`{"alarm_gif_seconds": 0}`:                 "alarm_gif_seconds",
		`{"alarm_gif_fps": 0}`:                     "alarm_gif_fps",
		`{"alarm_gif_fps": 99}`:                    "alarm_gif_fps",
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

func TestSavedConfigSurvivesRestart(t *testing.T) {
	path := t.TempDir() + "/settings.json"
	mon, stream, live, names := newTestSettings()
	if err := applyConfig(`{"detector": "difference", "alarm_gif_seconds": 8, "snap_jpeg_quality": 50, "paused": true}`, mon, stream, live, names); err != nil {
		t.Fatal(err)
	}
	if err := saveConfig(path, currentConfig(mon, stream, live)); err != nil {
		t.Fatal(err)
	}

	// a new run starts from the defaults and loads the file over them
	mon2, stream2, live2, _ := newTestSettings()
	found, err := loadSavedConfig(path, mon2, stream2, live2, names)
	if err != nil || !found {
		t.Fatalf("found %v, error %v", found, err)
	}
	if mon2.Detector() != detection.NameDifference || live2.Get().AlarmGIFSpan != 8*time.Second || live2.Get().SnapJPEGQuality != 50 {
		t.Errorf("the saved settings were not applied: %s", configJSON(currentConfig(mon2, stream2, live2)))
	}
	if mon2.Paused() {
		t.Error("the camera came back paused: paused must not be saved")
	}
}

func TestLoadSavedConfigMissingOrInvalid(t *testing.T) {
	dir := t.TempDir()
	mon, stream, live, names := newTestSettings()
	before := configJSON(currentConfig(mon, stream, live))

	if found, err := loadSavedConfig(dir+"/none.json", mon, stream, live, names); found || err != nil {
		t.Errorf("missing file: found %v, error %v", found, err)
	}

	bad := dir + "/bad.json"
	if err := os.WriteFile(bad, []byte(`{"snap_jpeg_quality": 0}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if found, err := loadSavedConfig(bad, mon, stream, live, names); !found || err == nil {
		t.Errorf("invalid file: found %v, error %v", found, err)
	}
	if after := configJSON(currentConfig(mon, stream, live)); after != before {
		t.Error("an invalid file changed the settings")
	}
}

func TestResetConfig(t *testing.T) {
	path := t.TempDir() + "/settings.json"
	mon, stream, live, names := newTestSettings()
	defaults := currentConfig(mon, stream, live)

	if err := applyConfig(`{"detector": "difference", "stack_frames": 400, "alarm_gif_fps": 4, "paused": true}`, mon, stream, live, names); err != nil {
		t.Fatal(err)
	}
	if err := saveConfig(path, currentConfig(mon, stream, live)); err != nil {
		t.Fatal(err)
	}
	if err := resetConfig(path, defaults, mon, stream, live, names); err != nil {
		t.Fatal(err)
	}

	got := currentConfig(mon, stream, live)
	want := defaults
	want.Paused = true // /reset_config doesn't unpause
	if configJSON(got) != configJSON(want) {
		t.Errorf("after the reset:\n%s\nwant:\n%s", configJSON(got), configJSON(want))
	}
	if _, err := os.Stat(path); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("the settings file is still there: %v", err)
	}
	// resetting with no file is fine
	if err := resetConfig(path, defaults, mon, stream, live, names); err != nil {
		t.Errorf("reset without a file: %v", err)
	}
}
