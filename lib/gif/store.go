package gif

/*
#include <stdint.h>
#include <stddef.h>

void *gif_store_new(const uint8_t *root, size_t root_len, uint64_t max, uint8_t **msg, size_t *msg_len);
void gif_store_free(void *store);
int32_t gif_store_save(const void *store, const uint8_t *code, size_t code_len, const uint8_t *data, size_t data_len, uint8_t **msg, size_t *msg_len);
int32_t gif_store_get(const void *store, const uint8_t *code, size_t code_len, uint8_t **out, size_t *out_len);
void gif_free(uint8_t *ptr, size_t len);
*/
import "C"

import (
	"crypto/rand"
	"encoding/hex"
	"errors"
	"runtime"
	"unsafe"

	_ "security_camera/lib/native"
)

// Statuses returned by the store functions in rust-lib/gif/src/ffi.rs.
const (
	storeOK          = 0
	storeNotFound    = 1
	storeInvalidCode = 2
)

// ErrNotFound is returned by Get for a code that was never saved or has been
// deleted to make room.
var ErrNotFound = errors.New("gif: no such gif")

// Store saves GIFs under a code and keeps their total size at or below max.
// It is implemented in Rust, see rust-lib/gif/src/store.rs.
type Store struct {
	rust *rustStore
}

// rustStore owns the store in Rust: the finalizer is on this and not on
// Store, so a Store can be part of a bigger struct.
type rustStore struct{ p unsafe.Pointer }

// NewStore opens the store in directory root, creating it if needed. max is the
// largest total size of the GIFs in bytes.
func NewStore(root string, max int64) (*Store, error) {
	var msg *C.uint8_t
	var msgLen C.size_t
	p := C.gif_store_new(cbytes(root), C.size_t(len(root)), C.uint64_t(max0(max)), &msg, &msgLen)
	if p == nil {
		return nil, takeError(msg, msgLen)
	}
	r := &rustStore{p: p}
	runtime.SetFinalizer(r, func(r *rustStore) { C.gif_store_free(r.p) })
	return &Store{rust: r}, nil
}

// NewCode returns a random code to save a GIF under: six hex digits, short
// enough to type in a Telegram command.
func NewCode() (string, error) {
	b := make([]byte, 3)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}

// Save stores data under code, then deletes the oldest GIFs until the total
// size is within the limit. A GIF bigger than the limit is deleted at once.
func (s *Store) Save(code string, data []byte) error {
	var msg *C.uint8_t
	var msgLen C.size_t
	var d *C.uint8_t
	if len(data) > 0 {
		d = (*C.uint8_t)(unsafe.Pointer(&data[0]))
	}
	st := C.gif_store_save(s.rust.p, cbytes(code), C.size_t(len(code)), d, C.size_t(len(data)), &msg, &msgLen)
	runtime.KeepAlive(s)
	if st == storeOK {
		return nil
	}
	return takeError(msg, msgLen)
}

// Get returns the GIF saved under code, in any letter case.
func (s *Store) Get(code string) ([]byte, error) {
	var out *C.uint8_t
	var n C.size_t
	st := C.gif_store_get(s.rust.p, cbytes(code), C.size_t(len(code)), &out, &n)
	runtime.KeepAlive(s)
	switch st {
	case storeOK:
		data := C.GoBytes(unsafe.Pointer(out), C.int(n))
		C.gif_free(out, n)
		return data, nil
	case storeNotFound:
		C.gif_free(out, n)
		return nil, ErrNotFound
	}
	return nil, takeError(out, n)
}

// takeError turns an error text from Rust into an error, and frees it.
func takeError(msg *C.uint8_t, n C.size_t) error {
	text := C.GoStringN((*C.char)(unsafe.Pointer(msg)), C.int(n))
	C.gif_free(msg, n)
	return errors.New(text)
}

// cbytes is the address of the first byte of s, or nil if there is none.
func cbytes(s string) *C.uint8_t {
	if s == "" {
		return nil
	}
	return (*C.uint8_t)(unsafe.Pointer(unsafe.StringData(s)))
}

func max0(n int64) int64 { return max(n, 0) }
