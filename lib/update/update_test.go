package update

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func script(t *testing.T, body string) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "update")
	if err := os.WriteFile(p, []byte("#!/bin/sh\n"+body), 0o755); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestBuild(t *testing.T) {
	out, err := Build(context.Background(), script(t, "echo 'updated a -> b'\n"), time.Minute)
	if err != nil || out != "updated a -> b" {
		t.Fatalf("got %q, %v", out, err)
	}
}

func TestBuildFailure(t *testing.T) {
	out, err := Build(context.Background(), script(t, "echo boom >&2\nexit 1\n"), time.Minute)
	if err == nil || !strings.Contains(out, "boom") {
		t.Fatalf("got %q, %v", out, err)
	}
}

func TestBuildTimeout(t *testing.T) {
	if _, err := Build(context.Background(), script(t, "sleep 5\n"), 100*time.Millisecond); err == nil {
		t.Fatal("slow script was not stopped")
	}
}

func TestTail(t *testing.T) {
	if got := Tail("abcdef", 3); got != "...def" {
		t.Fatalf("got %q", got)
	}
	if got := Tail("abc", 3); got != "abc" {
		t.Fatalf("got %q", got)
	}
}
