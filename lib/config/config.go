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

// StreamHistory is how many past frames are passed to the stream handlers.
const StreamHistory = 10

// GraphAddr is the address the live graph is served on.
const GraphAddr = ":8080"

// SnapJPEGQuality is the JPEG quality of the /snap photos.
const SnapJPEGQuality = 85

// UpdateScript is the script /update runs to pull and rebuild the program.
const UpdateScript = "./update"

// UpdateTimeout is how long the update script may run.
const UpdateTimeout = 10 * time.Minute
