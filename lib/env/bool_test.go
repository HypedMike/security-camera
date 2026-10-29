package env

import "testing"

func TestBool(t *testing.T) {
	for value, want := range map[string]bool{"1": true, "true": true, "TRUE": true, "0": false, "false": false, "": false, "nonsense": false} {
		t.Setenv("ENV_BOOL_TEST", value)
		if got := Bool("ENV_BOOL_TEST"); got != want {
			t.Errorf("Bool(%q) = %v, want %v", value, got, want)
		}
	}
}
