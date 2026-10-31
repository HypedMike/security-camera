//go:build rust

// Package native links the Rust library built in rust-lib/. The packages
// that call it import this one, so the linker flags are given only once.
package native

/*
#cgo LDFLAGS: -L${SRCDIR}/../../rust-lib/target/release -lnative
#cgo linux LDFLAGS: -lm -ldl -lpthread
*/
import "C"
