//go:build !rust

package gif

import "testing"

func TestUseRustFailsWithoutRust(t *testing.T) {
	if err := UseRust(true); err == nil {
		t.Fatal("UseRust(true) worked in a build without Rust")
	}
	if err := UseRust(false); err != nil {
		t.Fatal(err)
	}
}
