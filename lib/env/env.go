package env

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"
	"time"
)

// Bool reports whether the variable is set to a true value (1, true, ...).
func Bool(key string) bool {
	v, _ := strconv.ParseBool(os.Getenv(key))
	return v
}

// Load reads KEY=VALUE lines from path into the process environment.
// Blank lines and lines starting with # are ignored, and variables that are
// already set to a non-empty value are not overridden. A missing file is not an error.
func Load(path string) error {
	f, err := os.Open(path)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	defer f.Close()

	sc := bufio.NewScanner(f)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		key, value, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}
		key = strings.TrimSpace(key)
		value = strings.Trim(strings.TrimSpace(value), `"'`)
		if os.Getenv(key) == "" {
			os.Setenv(key, value)
		}
	}
	return sc.Err()
}

// Var describes a required variable that Ensure asks for when it is missing.
type Var struct {
	Key    string
	Prompt string
	// Validate, if set, rejects invalid input; the user is asked again.
	Validate func(string) error
}

// Ensure asks on out/in for every variable that is unset or empty, then sets
// it and saves it in the file at path (created with mode 0600, since it
// holds secrets) so it is found on the next start.
func Ensure(path string, in io.Reader, out io.Writer, vars ...Var) error {
	r := bufio.NewReader(in)
	for _, v := range vars {
		if os.Getenv(v.Key) != "" {
			continue
		}
		var value string
		for {
			fmt.Fprintf(out, "%s (%s): ", v.Prompt, v.Key)
			line, err := r.ReadString('\n')
			value = strings.TrimSpace(line)
			if value == "" && err != nil {
				return fmt.Errorf("env: %s is not set and could not be read: %w", v.Key, err)
			}
			if value == "" {
				continue
			}
			if v.Validate != nil {
				if verr := v.Validate(value); verr != nil {
					fmt.Fprintln(out, "invalid:", verr)
					continue
				}
			}
			break
		}
		if err := Save(path, v.Key, value); err != nil {
			return err
		}
		os.Setenv(v.Key, value)
	}
	return nil
}

// Save sets key=value in the file at path: lines already defining key are
// replaced, otherwise a line is appended. The file is created with mode 0600.
func Save(path, key, value string) error {
	data, err := os.ReadFile(path)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	var lines []string
	if len(data) > 0 {
		lines = strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	}
	line := key + "=" + value
	found := false
	out := lines[:0]
	for _, l := range lines {
		k, _, ok := strings.Cut(strings.TrimSpace(l), "=")
		if ok && strings.TrimSpace(k) == key {
			if !found {
				out = append(out, line)
			}
			found = true
			continue
		}
		out = append(out, l)
	}
	if !found {
		out = append(out, line)
	}
	return os.WriteFile(path, []byte(strings.Join(out, "\n")+"\n"), 0o600)
}

// Choose lists options on out and reads the number of one from in. If nothing
// is entered within timeout (or the input is closed), the first option is
// returned. Invalid entries ask again; the timeout restarts only with the
// whole call, not per entry.
func Choose(in io.Reader, out io.Writer, prompt string, options []string, timeout time.Duration) int {
	fmt.Fprintln(out, prompt)
	for i, o := range options {
		fmt.Fprintf(out, "  %d) %s\n", i+1, o)
	}
	lines := make(chan string)
	go func() { // may outlive the call if the timeout fires; it only blocks on the read
		r := bufio.NewReader(in)
		for {
			line, err := r.ReadString('\n')
			if strings.TrimSpace(line) != "" {
				lines <- strings.TrimSpace(line)
			}
			if err != nil {
				close(lines)
				return
			}
		}
	}()
	deadline := time.After(timeout)
	for {
		fmt.Fprintf(out, "Choose 1-%d [1 in %ds]: ", len(options), int(timeout.Seconds()))
		select {
		case line, ok := <-lines:
			if !ok {
				fmt.Fprintln(out)
				return 0
			}
			if n, err := strconv.Atoi(line); err == nil && n >= 1 && n <= len(options) {
				return n - 1
			}
			fmt.Fprintln(out, "invalid choice")
		case <-deadline:
			fmt.Fprintf(out, "\nno answer, using %s\n", options[0])
			return 0
		}
	}
}
