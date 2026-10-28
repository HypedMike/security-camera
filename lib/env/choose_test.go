package env

import (
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestChoose(t *testing.T) {
	opts := []string{"a", "b", "c"}
	if got := Choose(strings.NewReader("x\n9\n2\n"), io.Discard, "p", opts, time.Second); got != 1 {
		t.Errorf("valid choice = %d", got)
	}
	if got := Choose(strings.NewReader(""), io.Discard, "p", opts, time.Second); got != 0 {
		t.Errorf("closed input = %d", got)
	}
	pr, pw := io.Pipe() // never written to: must time out
	defer pw.Close()
	start := time.Now()
	if got := Choose(pr, io.Discard, "p", opts, 50*time.Millisecond); got != 0 || time.Since(start) > time.Second {
		t.Errorf("timeout = %d after %v", got, time.Since(start))
	}
}

func TestSaveReplacesAndAppends(t *testing.T) {
	path := filepath.Join(t.TempDir(), ".env")
	os.WriteFile(path, []byte("# c\nA=1\nB=2\nA=3\n"), 0o644)
	if err := Save(path, "A", "9"); err != nil {
		t.Fatal(err)
	}
	if err := Save(path, "C", "4"); err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(path)
	if string(got) != "# c\nA=9\nB=2\nC=4\n" {
		t.Errorf("got %q", got)
	}
}
