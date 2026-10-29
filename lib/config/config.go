// Package config holds the fixed settings of the security camera.
package config

import "time"

// AlarmCooldown is the minimum time between two Telegram notifications.
const AlarmCooldown = 30 * time.Second

// GIFFrameDelay is the pause between GIF frames, in hundredths of a second.
const GIFFrameDelay = 10

// CameraChoiceTimeout is how long the camera prompt waits before using the first camera.
const CameraChoiceTimeout = 5 * time.Second
