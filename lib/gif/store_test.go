package gif

import (
	"bytes"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestSaveGet(t *testing.T) {
	s, err := NewStore(t.TempDir(), 100)
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Save("abc123", []byte("data")); err != nil {
		t.Fatal(err)
	}
	got, err := s.Get("ABC123")
	if err != nil || !bytes.Equal(got, []byte("data")) {
		t.Fatalf("Get = %q, %v", got, err)
	}
	if _, err := s.Get("ffffff"); !errors.Is(err, ErrNotFound) {
		t.Errorf("missing code: err = %v, want ErrNotFound", err)
	}
}

func TestInvalidCodes(t *testing.T) {
	s, _ := NewStore(t.TempDir(), 100)
	for _, code := range []string{"", "../x", "abc12", "abc1234", "ghijkl", "..%2f..", "a/b123"} {
		if err := s.Save(code, nil); err == nil {
			t.Errorf("Save(%q) succeeded", code)
		}
		if _, err := s.Get(code); !errors.Is(err, ErrNotFound) {
			t.Errorf("Get(%q) err = %v, want ErrNotFound", code, err)
		}
	}
}

func TestEvictsOldestFirst(t *testing.T) {
	dir := t.TempDir()
	s, _ := NewStore(dir, 25)
	base := time.Now().Add(-time.Hour)
	for i, code := range []string{"000001", "000002", "000003"} {
		if err := s.Save(code, bytes.Repeat([]byte("x"), 10)); err != nil {
			t.Fatal(err)
		}
		// give each file a distinct age, the filesystem clock may be coarse
		mod := base.Add(time.Duration(i) * time.Minute)
		if err := os.Chtimes(filepath.Join(dir, code+".gif"), mod, mod); err != nil {
			t.Fatal(err)
		}
	}
	// 30 bytes > 25: the next save pushes out the oldest ones
	if err := s.Save("000004", bytes.Repeat([]byte("x"), 10)); err != nil {
		t.Fatal(err)
	}
	for code, want := range map[string]bool{"000001": false, "000002": false, "000003": true, "000004": true} {
		_, err := s.Get(code)
		if (err == nil) != want {
			t.Errorf("Get(%s): err = %v, want present = %v", code, err, want)
		}
	}
}

func TestOversizeGIFIsDropped(t *testing.T) {
	s, _ := NewStore(t.TempDir(), 5)
	if err := s.Save("000001", make([]byte, 10)); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Get("000001"); !errors.Is(err, ErrNotFound) {
		t.Errorf("err = %v, want ErrNotFound", err)
	}
}

func TestNewCode(t *testing.T) {
	c, err := NewCode()
	if err != nil || !validCode(c) {
		t.Fatalf("NewCode = %q, %v", c, err)
	}
}
