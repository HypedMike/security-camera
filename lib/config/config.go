// Package config holds the fixed settings of the security camera.
package config

import "time"

// AlarmCooldown is the minimum time between two Telegram notifications.
const AlarmCooldown = 30 * time.Second

// GIFFrameDelay is the pause between GIF frames, in hundredths of a second.
const GIFFrameDelay = 10

// CameraChoiceTimeout is how long the camera prompt waits before using the first camera.
const CameraChoiceTimeout = 5 * time.Second

// EnvFile is the file the settings are loaded from and saved to.
const EnvFile = ".env"

// StreamHistory is how many past frames the stack keeps by default (n).
const StreamHistory = 300

// GIFFrames is how many of the latest frames an alarm GIF contains by default (m).
const GIFFrames = 150

// DefaultDetector is the motion detector used when DETECTOR is not set.
const DefaultDetector = "background"

// BackgroundThreshold is the default alarm threshold of the background
// detector: the area of the largest moving blob as a fraction of the frame.
const BackgroundThreshold = 0.003

// DifferenceThreshold is the default alarm threshold of the difference
// detector: the average change between consecutive frames.
const DifferenceThreshold = 0.05

// DetectionFrames is how many of the latest frames the difference detector
// computes the difference on by default (t).
const DetectionFrames = 30

// GraphAddr is the address the live graph is served on.
const GraphAddr = ":8080"

// SnapJPEGQuality is the JPEG quality of the /snap photos.
const SnapJPEGQuality = 85

// UpdateScript is the script /update runs to pull and rebuild the program.
const UpdateScript = "./update"

// UpdateTimeout is how long the update script may run.
const UpdateTimeout = 10 * time.Minute
