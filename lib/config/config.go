// Package config holds the fixed settings of the security camera.
package config

import "time"

// AlarmCooldown is the minimum time between two Telegram notifications.
const AlarmCooldown = 5 * time.Second

// AlarmDelay is how long the program waits after it detects motion before it
// sends the alarm, so the GIF also shows what happens next: at the first
// detection the thing that moved is often not fully in the frame yet.
const AlarmDelay = 3 * time.Second

// Every alarm makes two GIFs from the same frames. The low quality one is
// sent with the alarm and is tiny, so it arrives fast even on a poor
// connection; the HD one is kept on disk and sent by /hd <code>.
//
// The delay is in hundredths of a second between frames. On a 30 fps camera
// a step of 3 with a delay of 10 plays at real speed, and so do 6 and 20.

// HDGIFFrameStep keeps one frame out of every HDGIFFrameStep of the last
// GIFFrames: 150 frames become 75, about 5 seconds long.
const HDGIFFrameStep = 2

// HDGIFFrameDelay is the pause between HD GIF frames.
const HDGIFFrameDelay = 7

// HDGIFWidth is the largest width of the HD GIF frames: the full 640x480.
const HDGIFWidth = 640

// HDGIFDither smooths the HD GIF colours, at the price of a file up to about
// 3 times bigger.
const HDGIFDither = true

// LowGIFFrameStep is how many HD GIF frames are skipped to make the low
// quality GIF, which is therefore one frame in HDGIFFrameStep*LowGIFFrameStep
// of the camera: 75 frames become 25, still about 5 seconds long.
const LowGIFFrameStep = 3

// LowGIFFrameDelay is the pause between low quality GIF frames.
const LowGIFFrameDelay = 20

// LowGIFWidth is the largest width of the low quality GIF frames: 160x120.
const LowGIFWidth = 160

// GIFDir is the directory the HD GIFs are saved in.
const GIFDir = "gifs"

// GIFStorageLimit is the largest total size in bytes of the saved HD GIFs
// (1 GB). Past it the oldest GIFs are deleted.
const GIFStorageLimit = 1 << 30

// CameraChoiceTimeout is how long the camera prompt waits before using the first camera.
const CameraChoiceTimeout = 5 * time.Second

// EnvFile is the file the settings are loaded from and saved to.
const EnvFile = ".env"

// StreamHistory is how many past frames the stack keeps by default (n).
const StreamHistory = 300

// GIFFrames is how many of the latest frames an alarm GIF covers by default
// (m), before GIFFrameStep drops some of them.
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
