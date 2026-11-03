package my_video

import (
	"slices"
	"testing"
)

var devs = []Device{
	{ID: "x1", Label: "usb-Cam-video-index0;video0"},
	{ID: "x2", Label: "usb-Cam-video-index1;video1"},
}

func TestFindByLabelOrID(t *testing.T) {
	if d, ok := find(devs, "usb-Cam-video-index1;video1"); !ok || d.ID != "x2" {
		t.Errorf("by label: %v %v", d, ok)
	}
	// the id the driver gave in this run
	if d, ok := find(devs, "x1"); !ok || d.Label != "usb-Cam-video-index0;video0" {
		t.Errorf("by id: %v %v", d, ok)
	}
	if _, ok := find(devs, "usb-Other-video-index0;video0"); ok {
		t.Error("unknown camera found")
	}
}

func TestOpenOrder(t *testing.T) {
	if got := openOrder(devs, "x2"); !slices.Equal(got, []string{"x2", "x1"}) {
		t.Errorf("chosen first, then the others: %v", got)
	}
	if got := openOrder(devs, ""); !slices.Equal(got, []string{"", "x1", "x2"}) {
		t.Errorf("nothing chosen: %v", got)
	}
	if got := openOrder(nil, ""); !slices.Equal(got, []string{""}) {
		t.Errorf("no cameras: the default one must still be tried: %v", got)
	}
	if got := openOrder(devs, "unknown"); got[0] != "unknown" || len(got) != 3 {
		t.Errorf("unknown id: %v", got)
	}
}
