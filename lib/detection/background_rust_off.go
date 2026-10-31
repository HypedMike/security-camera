//go:build !rust

package detection

// RustAvailable reports whether this binary was built with the Rust
// detectors (go build -tags rust).
const RustAvailable = false

// BackgroundRust is not available in this build: see background_rust.go.
type BackgroundRust struct{ Detector }

// NewBackgroundRust returns nil in builds without -tags rust.
func NewBackgroundRust() *BackgroundRust { return nil }
