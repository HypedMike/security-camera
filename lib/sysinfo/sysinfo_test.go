package sysinfo

import (
	"strings"
	"testing"
)

func TestCPUTimes(t *testing.T) {
	busy, total, ok := cpuTimes("cpu  100 10 50 800 40 0 0 0 0 0\ncpu0 1 2 3 4 5\n")
	if !ok || busy != 160 || total != 1000 {
		t.Fatalf("got busy=%d total=%d ok=%v", busy, total, ok)
	}
	if _, _, ok := cpuTimes("garbage"); ok {
		t.Fatal("garbage parsed")
	}
}

func TestProcTimes(t *testing.T) {
	stat := "42 (my (cam) prog) S 1 2 3 4 5 6 7 8 9 10 111 222 0 0 20"
	got, ok := procTimes(stat)
	if !ok || got != 333 {
		t.Fatalf("got %d ok=%v", got, ok)
	}
}

func TestKvKB(t *testing.T) {
	m := kvKB("MemTotal:  2048 kB\nMemFree:   1 kB\nnope\n")
	if m["MemTotal"] != 2048 || m["MemFree"] != 1 {
		t.Fatalf("got %v", m)
	}
}

func TestReport(t *testing.T) {
	r := Report(".")
	for _, want := range []string{"System", "CPU", "Disk", "Security camera process"} {
		if !strings.Contains(r, want) {
			t.Errorf("report misses %q:\n%s", want, r)
		}
	}
}
