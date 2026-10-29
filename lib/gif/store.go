package gif

import (
	"crypto/rand"
	"encoding/hex"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"

	"security_camera/lib/storage"
)

const ext = ".gif"

// ErrNotFound is returned by Get for a code that was never saved or has been
// deleted to make room.
var ErrNotFound = errors.New("gif: no such gif")

// Store saves GIFs under a code and keeps their total size at or below max.
type Store struct {
	disk *storage.LocalStorage
	root string
	max  int64

	mu sync.Mutex
}

// NewStore opens the store in directory root, creating it if needed. max is the
// largest total size of the GIFs in bytes.
func NewStore(root string, max int64) (*Store, error) {
	disk, err := storage.NewLocalStorage(root)
	if err != nil {
		return nil, err
	}
	return &Store{disk: disk, root: root, max: max}, nil
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

// validCode reports whether code looks like one NewCode returns, which also
// keeps a user supplied code from naming any other file.
func validCode(code string) bool {
	if len(code) != 6 {
		return false
	}
	for _, c := range code {
		if !('0' <= c && c <= '9' || 'a' <= c && c <= 'f') {
			return false
		}
	}
	return true
}

// Save stores data under code, then deletes the oldest GIFs until the total
// size is within the limit. A GIF bigger than the limit is deleted at once.
func (s *Store) Save(code string, data []byte) error {
	if !validCode(code) {
		return errors.New("gif: invalid code")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.disk.Save(code+ext, data); err != nil {
		return err
	}
	return s.evict()
}

// Get returns the GIF saved under code, in any letter case.
func (s *Store) Get(code string) ([]byte, error) {
	code = strings.ToLower(code)
	if !validCode(code) {
		return nil, ErrNotFound
	}
	data, err := s.disk.Get(code + ext)
	if errors.Is(err, os.ErrNotExist) {
		return nil, ErrNotFound
	}
	return data, err
}

type file struct {
	name string
	size int64
	mod  time.Time
}

// evict deletes the oldest GIFs until the total size is within max.
func (s *Store) evict() error {
	entries, err := os.ReadDir(s.root)
	if err != nil {
		return err
	}
	var files []file
	var total int64
	for _, e := range entries {
		if e.IsDir() || filepath.Ext(e.Name()) != ext {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		files = append(files, file{e.Name(), info.Size(), info.ModTime()})
		total += info.Size()
	}
	slices.SortFunc(files, func(a, b file) int {
		if c := a.mod.Compare(b.mod); c != 0 {
			return c
		}
		return strings.Compare(a.name, b.name)
	})
	for _, f := range files {
		if total <= s.max {
			break
		}
		if err := s.disk.Delete(f.name); err != nil {
			return err
		}
		total -= f.size
	}
	return nil
}
