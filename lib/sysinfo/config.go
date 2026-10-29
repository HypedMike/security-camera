package sysinfo

import "time"

// sampleWindow is how long CPU usage is measured over.
const sampleWindow = 300 * time.Millisecond
