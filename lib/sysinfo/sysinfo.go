// Package sysinfo reports the state of the machine and of this process.
// It reads /proc and /sys, so most values are only available on Linux; what
// cannot be read is left out of the report instead of failing it.
package sysinfo

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// sampleWindow is how long CPU usage is measured over.
const sampleWindow = 300 * time.Millisecond

var startTime = time.Now()

// cpuTimes returns the busy and total jiffies of the whole machine from the
// first line of /proc/stat.
func cpuTimes(stat string) (busy, total uint64, ok bool) {
	line, _, _ := strings.Cut(stat, "\n")
	f := strings.Fields(line)
	if len(f) < 5 || f[0] != "cpu" {
		return 0, 0, false
	}
	var idle uint64
	for i, s := range f[1:] {
		v, err := strconv.ParseUint(s, 10, 64)
		if err != nil {
			return 0, 0, false
		}
		if i >= 8 { // guest times are already counted in user/nice
			break
		}
		total += v
		if i == 3 || i == 4 { // idle, iowait
			idle += v
		}
	}
	return total - idle, total, true
}

// procTimes returns this process's utime+stime in jiffies from /proc/self/stat.
func procTimes(stat string) (uint64, bool) {
	// the command name is in parentheses and may contain spaces
	i := strings.LastIndexByte(stat, ')')
	if i < 0 {
		return 0, false
	}
	f := strings.Fields(stat[i+1:])
	if len(f) < 13 { // utime and stime are fields 14 and 15 of the whole line
		return 0, false
	}
	u, err1 := strconv.ParseUint(f[11], 10, 64)
	s, err2 := strconv.ParseUint(f[12], 10, 64)
	if err1 != nil || err2 != nil {
		return 0, false
	}
	return u + s, true
}

// kvKB parses "Key:   123 kB" lines into kilobytes by key.
func kvKB(s string) map[string]uint64 {
	m := map[string]uint64{}
	for _, line := range strings.Split(s, "\n") {
		k, v, ok := strings.Cut(line, ":")
		if !ok {
			continue
		}
		f := strings.Fields(v)
		if len(f) == 0 {
			continue
		}
		if n, err := strconv.ParseUint(f[0], 10, 64); err == nil {
			m[k] = n
		}
	}
	return m
}

func readFile(path string) string {
	b, _ := os.ReadFile(path)
	return strings.TrimSpace(string(b))
}

// temperatures returns "name: 45.0°C" for every thermal zone, falling back to
// hwmon sensors when there are none.
func temperatures() []string {
	var out []string
	add := func(name, file string) {
		v, err := strconv.ParseFloat(readFile(file), 64)
		if err != nil {
			return
		}
		out = append(out, fmt.Sprintf("%s %.1f°C", name, v/1000))
	}
	zones, _ := filepath.Glob("/sys/class/thermal/thermal_zone*/temp")
	for _, f := range zones {
		name := readFile(filepath.Join(filepath.Dir(f), "type"))
		if name == "" {
			name = filepath.Base(filepath.Dir(f))
		}
		add(name, f)
	}
	if len(out) > 0 {
		return out
	}
	hw, _ := filepath.Glob("/sys/class/hwmon/hwmon*/temp*_input")
	for _, f := range hw {
		name := readFile(filepath.Join(filepath.Dir(f), "name"))
		if name == "" {
			name = filepath.Base(filepath.Dir(f))
		}
		add(name, f)
	}
	return out
}

func mb(kb uint64) string { return fmt.Sprintf("%.0f MB", float64(kb)/1024) }

func gb(b uint64) string { return fmt.Sprintf("%.1f GB", float64(b)/(1<<30)) }

func pct(part, whole uint64) float64 {
	if whole == 0 {
		return 0
	}
	return 100 * float64(part) / float64(whole)
}

func formatDuration(d time.Duration) string {
	d = d.Round(time.Second)
	days := d / (24 * time.Hour)
	d -= days * 24 * time.Hour
	if days > 0 {
		return fmt.Sprintf("%dd %s", days, d)
	}
	return d.String()
}

// Report returns a multi-line, human readable summary of the machine and of
// this process. It blocks for a fraction of a second to measure CPU usage.
// dir is the directory whose disk usage is reported.
func Report(dir string) string {
	var b strings.Builder
	section := func(title string) { fmt.Fprintf(&b, "\n%s\n", title) }
	line := func(format string, args ...any) { fmt.Fprintf(&b, "  "+format+"\n", args...) }

	// sample CPU usage over a short window, for the machine and for this process
	stat0 := readFile("/proc/stat")
	proc0, procOK := procTimes(readFile("/proc/self/stat"))
	time.Sleep(sampleWindow)
	busy0, total0, ok0 := cpuTimes(stat0)
	busy1, total1, ok1 := cpuTimes(readFile("/proc/stat"))
	proc1, procOK1 := procTimes(readFile("/proc/self/stat"))

	host, _ := os.Hostname()
	section("System")
	line("host: %s (%s/%s)", host, runtime.GOOS, runtime.GOARCH)
	if up, _, ok := strings.Cut(readFile("/proc/uptime"), " "); ok {
		if s, err := strconv.ParseFloat(up, 64); err == nil {
			line("uptime: %s", formatDuration(time.Duration(s*float64(time.Second))))
		}
	}
	if la := strings.Fields(readFile("/proc/loadavg")); len(la) >= 3 {
		line("load average: %s %s %s (%d cores)", la[0], la[1], la[2], runtime.NumCPU())
	}

	section("CPU")
	if ok0 && ok1 && total1 > total0 {
		line("usage: %.1f%%", pct(busy1-busy0, total1-total0))
	}
	if temps := temperatures(); len(temps) > 0 {
		for _, t := range temps {
			line("temperature: %s", t)
		}
	} else {
		line("temperature: not available")
	}

	if m := kvKB(readFile("/proc/meminfo")); m["MemTotal"] > 0 {
		section("Memory")
		used := m["MemTotal"] - m["MemAvailable"]
		line("ram: %s / %s (%.1f%%)", mb(used), mb(m["MemTotal"]), pct(used, m["MemTotal"]))
		if m["SwapTotal"] > 0 {
			swapUsed := m["SwapTotal"] - m["SwapFree"]
			line("swap: %s / %s (%.1f%%)", mb(swapUsed), mb(m["SwapTotal"]), pct(swapUsed, m["SwapTotal"]))
		}
	}

	var fs syscall.Statfs_t
	if err := syscall.Statfs(dir, &fs); err == nil {
		section("Disk")
		total := uint64(fs.Blocks) * uint64(fs.Bsize)
		free := uint64(fs.Bavail) * uint64(fs.Bsize)
		line("%s: %s free of %s (%.1f%% used)", dir, gb(free), gb(total), pct(total-free, total))
	}

	section("Security camera process")
	line("pid: %d, running for %s", os.Getpid(), formatDuration(time.Since(startTime)))
	if procOK && procOK1 && ok0 && ok1 && total1 > total0 {
		// jiffies of this process over jiffies of all cores: 100% means one full core
		share := float64(proc1-proc0) / float64(total1-total0) * float64(runtime.NumCPU())
		line("cpu: %.1f%% of one core", 100*share)
	}
	if s := kvKB(readFile("/proc/self/status")); s["VmRSS"] > 0 {
		line("memory (rss): %s", mb(s["VmRSS"]))
	}
	var ms runtime.MemStats
	runtime.ReadMemStats(&ms)
	line("go heap: %s in use, %s from the os", mb(ms.HeapAlloc/1024), mb(ms.Sys/1024))
	line("goroutines: %d", runtime.NumGoroutine())

	return strings.TrimSpace(b.String())
}
