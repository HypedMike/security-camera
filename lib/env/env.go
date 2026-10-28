package env

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"strings"
)

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
// it and appends it to the file at path (created with mode 0600, since it
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
		if err := save(path, v.Key, value); err != nil {
			return err
		}
		os.Setenv(v.Key, value)
	}
	return nil
}

func save(path, key, value string) error {
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	if _, err := fmt.Fprintf(f, "%s=%s\n", key, value); err != nil {
		f.Close()
		return err
	}
	return f.Close()
}
