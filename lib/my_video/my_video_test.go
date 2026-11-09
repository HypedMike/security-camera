package my_video

import "testing"

// Without hardware there is little to check, but the whole path to Rust and
// back (the list, the error message) must work. The logic is tested in
// rust-lib/my_video/.
func TestNoCamera(t *testing.T) {
	devs := Devices()
	if len(devs) > 0 {
		t.Skip("a camera is connected")
	}
	if _, err := OpenAny(Config{Width: 640, Height: 480, DeviceID: "nope"}); err == nil || err.Error() == "" {
		t.Errorf("opening a missing camera: error %v", err)
	}
	list := listDevices()
	defer list.Close()
	if list.len() != 0 || list.find("x") != -1 || list.find("") != -1 {
		t.Error("an empty list found something")
	}
}
