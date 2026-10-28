package env

import (
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

func TestEnsure(t *testing.T) {
	t.Setenv("ENSURE_SET", "already")
	t.Setenv("ENSURE_A", "")
	t.Setenv("ENSURE_B", "")
	path := filepath.Join(t.TempDir(), ".env")

	num := func(s string) error { _, err := strconv.ParseFloat(s, 64); return err }
	in := strings.NewReader("tok\nabc\n0.05\n")
	err := Ensure(path, in, io.Discard,
		Var{Key: "ENSURE_SET", Prompt: "set"},
		Var{Key: "ENSURE_A", Prompt: "a"},
		Var{Key: "ENSURE_B", Prompt: "b", Validate: num},
	)
	if err != nil {
		t.Fatal(err)
	}
	// "tok" fills A, then "abc" is rejected and "0.05" fills B
	if os.Getenv("ENSURE_A") != "tok" || os.Getenv("ENSURE_B") != "0.05" {
		t.Errorf("A=%q B=%q", os.Getenv("ENSURE_A"), os.Getenv("ENSURE_B"))
	}
	got, _ := os.ReadFile(path)
	if string(got) != "ENSURE_A=tok\nENSURE_B=0.05\n" {
		t.Errorf("saved %q", got)
	}
	if st, _ := os.Stat(path); st.Mode().Perm() != 0o600 {
		t.Errorf("mode %v", st.Mode().Perm())
	}

	// no input left: error instead of looping forever
	t.Setenv("ENSURE_C", "")
	if err := Ensure(path, strings.NewReader(""), io.Discard, Var{Key: "ENSURE_C", Prompt: "c"}); err == nil {
		t.Error("expected error on EOF")
	}
}
