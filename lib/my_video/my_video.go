// Package my_video opens the camera and reads its frames. The work is done in
// Rust, see rust-lib/my_video/: it captures with V4L2 on Linux and with
// AVFoundation on macOS, and decodes the frames to RGBA.
package my_video

/*
#include <stdint.h>
#include <stddef.h>

void *video_devices(void);
void video_devices_free(void *list);
size_t video_devices_len(const void *list);
size_t video_devices_id(const void *list, size_t i, uint8_t *buf, size_t cap);
size_t video_devices_label(const void *list, size_t i, uint8_t *buf, size_t cap);
ptrdiff_t video_devices_find(const void *list, const uint8_t *saved, size_t len);
void *video_open(const uint8_t *first, size_t first_len, uint32_t width, uint32_t height, size_t warmup, uint8_t *err, size_t err_cap, size_t *err_len);
const uint8_t *video_read(const void *video, uint32_t *width, uint32_t *height);
size_t video_error(const void *video, uint8_t *buf, size_t cap);
void video_stop(const void *video);
void video_close(void *video);
*/
import "C"

import (
	"errors"
	"image"
	"sync"
	"unsafe"

	_ "security_camera/lib/native"
)

type Config struct {
	Width  int
	Height int
	// WarmupFrames are read and discarded on open so auto-exposure can settle.
	WarmupFrames int
	// DeviceID selects the camera (see Devices); empty means the default one.
	DeviceID string
}

// Device is an available camera.
type Device struct {
	ID    string
	Label string
}

var DefaultConfig = Config{Width: 640, Height: 480, WarmupFrames: 30}

// deviceList is the list of cameras in Rust, freed with Close.
type deviceList struct{ p unsafe.Pointer }

func listDevices() deviceList { return deviceList{C.video_devices()} }

func (l deviceList) Close() { C.video_devices_free(l.p) }

func (l deviceList) len() int { return int(C.video_devices_len(l.p)) }

// text reads a string out of Rust with fill, which writes into a buffer and
// returns the full length of the string.
func text(fill func(buf *C.uint8_t, n C.size_t) C.size_t) string {
	buf := make([]byte, 256)
	for {
		n := int(fill((*C.uint8_t)(unsafe.Pointer(&buf[0])), C.size_t(len(buf))))
		if n <= len(buf) {
			return string(buf[:n])
		}
		buf = make([]byte, n)
	}
}

func (l deviceList) at(i int) Device {
	return Device{
		ID:    text(func(b *C.uint8_t, n C.size_t) C.size_t { return C.video_devices_id(l.p, C.size_t(i), b, n) }),
		Label: text(func(b *C.uint8_t, n C.size_t) C.size_t { return C.video_devices_label(l.p, C.size_t(i), b, n) }),
	}
}

// find returns the position of the camera CAMERA_ID names, or -1: by label, or
// by id for the ids that older versions saved (those only match within the run
// that saved them).
func (l deviceList) find(saved string) int {
	if saved == "" {
		return -1
	}
	return int(C.video_devices_find(l.p, (*C.uint8_t)(unsafe.Pointer(unsafe.StringData(saved))), C.size_t(len(saved))))
}

// Devices lists the cameras currently available, sorted by label. The driver
// can hand them out in a different order at every start, and the ids change
// too: the label is the only thing that stays the same.
func Devices() []Device {
	l := listDevices()
	defer l.Close()
	devs := make([]Device, l.len())
	for i := range devs {
		devs[i] = l.at(i)
	}
	return devs
}

// Video owns the camera and its stream of frames.
type Video struct {
	// mu keeps Close from freeing the camera while Read is using it.
	mu sync.RWMutex
	p  unsafe.Pointer // nil once closed
}

// OpenAny opens the camera cfg.DeviceID. A camera often shows up as several
// video nodes of which only one delivers images, so if that one can't be
// opened the other cameras are tried in turn, and a wrong choice does not stop
// the program. If none opens, it returns the first error.
func OpenAny(cfg Config) (*Video, error) {
	var first unsafe.Pointer
	if cfg.DeviceID != "" {
		first = unsafe.Pointer(unsafe.StringData(cfg.DeviceID))
	}
	var errBuf [512]byte
	var errLen C.size_t
	p := C.video_open((*C.uint8_t)(first), C.size_t(len(cfg.DeviceID)), C.uint32_t(cfg.Width), C.uint32_t(cfg.Height),
		C.size_t(cfg.WarmupFrames), (*C.uint8_t)(unsafe.Pointer(&errBuf[0])), C.size_t(len(errBuf)), &errLen)
	if p == nil {
		return nil, errors.New(string(errBuf[:min(int(errLen), len(errBuf))]))
	}
	return &Video{p: p}, nil
}

// Read waits for the next frame. The image is only valid until the next call
// to Read or Close, and release does nothing: it is there so a Video can be
// the source of a streaming.Stream. A frame the caller was too slow for is
// skipped.
func (v *Video) Read() (img image.Image, release func(), err error) {
	v.mu.RLock()
	defer v.mu.RUnlock()
	if v.p == nil {
		return nil, nil, errors.New("my_video: closed")
	}
	var w, h C.uint32_t
	pix := C.video_read(v.p, &w, &h)
	if pix == nil {
		msg := text(func(b *C.uint8_t, n C.size_t) C.size_t { return C.video_error(v.p, b, n) })
		return nil, nil, errors.New(msg)
	}
	return &image.RGBA{
		Pix:    unsafe.Slice((*uint8)(unsafe.Pointer(pix)), int(w)*int(h)*4),
		Stride: int(w) * 4,
		Rect:   image.Rect(0, 0, int(w), int(h)),
	}, func() {}, nil
}

// Close releases the camera. A Read that is waiting fails.
func (v *Video) Close() error {
	// make a waiting Read return, it holds the read lock
	if p := v.pointer(); p != nil {
		C.video_stop(p)
	}
	v.mu.Lock()
	defer v.mu.Unlock()
	if v.p != nil {
		C.video_close(v.p)
		v.p = nil
	}
	return nil
}

func (v *Video) pointer() unsafe.Pointer {
	v.mu.RLock()
	defer v.mu.RUnlock()
	return v.p
}
