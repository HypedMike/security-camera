// Package update pulls and rebuilds the program and restarts it in place.
package update

import (
	"context"
	"os"
	"os/exec"
	"strings"
	"syscall"
	"time"
)

// Build runs the update script and returns its combined output. The script
// must pull and build; a non-zero exit is reported as an error and leaves the
// running program untouched.
func Build(ctx context.Context, script string, timeout time.Duration) (string, error) {
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, script)
	// kill the whole group on timeout: git and go are children of the script
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.Cancel = func() error { return syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL) }
	cmd.WaitDelay = time.Second
	out, err := cmd.CombinedOutput()
	return strings.TrimSpace(string(out)), err
}

// Tail returns at most max bytes from the end of s, which is where the
// outcome of a long build log is.
func Tail(s string, max int) string {
	if len(s) <= max {
		return s
	}
	return "..." + s[len(s)-max:]
}

// executable returns the path of the running binary, even after a rebuild
// replaced it on disk (Linux then marks the old path as " (deleted)").
func executable() (string, error) {
	exe, err := os.Executable()
	return strings.TrimSuffix(exe, " (deleted)"), err
}

// Restart replaces the running process with a fresh copy of the (rebuilt)
// binary, same arguments and environment. It only returns on failure. Close
// the camera first: the new process needs to open it again.
func Restart() error {
	exe, err := executable()
	if err != nil {
		return err
	}
	return syscall.Exec(exe, os.Args, os.Environ())
}
