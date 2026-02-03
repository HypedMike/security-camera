# Security Camera with Motion Detection

A Go program that reads video from a webcam, detects motion, and calls an API endpoint when motion is detected.

## Features

- Real-time webcam video capture
- Motion detection using computer vision
- Automatic API endpoint notification when motion is detected
- Cooldown period to prevent API spam
- Visual feedback window showing camera feed
- Configurable sensitivity and detection parameters

## Prerequisites

### Install OpenCV

#### macOS

```bash
brew install opencv
```

#### Linux (Ubuntu/Debian)

```bash
sudo apt-get update
sudo apt-get install libopencv-dev
```

#### Windows

Download and install OpenCV from the official website or use package managers like chocolatey.

### Install Go Dependencies

```bash
go mod download
```

## Configuration

Edit the constants in `main.go` to customize behavior:

- `MinimumArea`: Minimum pixel area for motion detection (default: 3000)
- `DeltaThreshold`: Sensitivity of motion detection (default: 25)
- `APIEndpoint`: URL to call when motion is detected (default: "http://localhost:8080/motion-detected")
- `CooldownPeriod`: Time between API calls (default: 5 seconds)

## Usage

### Run the Security Camera

```bash
go run main.go
```

The program will:

1. Open your default webcam (device ID 0)
2. Display a window showing the camera feed
3. Monitor for motion continuously
4. Call the configured API endpoint when motion is detected
5. Display "MOTION DETECTED!" on the video feed

Press **ESC** to exit the program.

### Test API Endpoint

For testing, you can create a simple server to receive motion notifications:

```go
// test-server.go
package main

import (
    "encoding/json"
    "log"
    "net/http"
    "time"
)

type MotionEvent struct {
    Timestamp time.Time `json:"timestamp"`
    Message   string    `json:"message"`
}

func motionHandler(w http.ResponseWriter, r *http.Request) {
    if r.Method != http.MethodPost {
        http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
        return
    }

    var event MotionEvent
    if err := json.NewDecoder(r.Body).Decode(&event); err != nil {
        http.Error(w, err.Error(), http.StatusBadRequest)
        return
    }

    log.Printf("Motion detected at %s: %s\n", event.Timestamp.Format(time.RFC3339), event.Message)
    w.WriteHeader(http.StatusOK)
    w.Write([]byte("Motion event received"))
}

func main() {
    http.HandleFunc("/motion-detected", motionHandler)
    log.Println("Test server listening on :8080")
    log.Fatal(http.ListenAndServe(":8080", nil))
}
```

Run the test server:

```bash
go run test-server.go
```

## How It Works

1. **Video Capture**: Captures frames from the webcam using GoCV
2. **Motion Detection**:
   - Converts frames to grayscale
   - Applies Gaussian blur to reduce noise
   - Computes the absolute difference between consecutive frames
   - Applies threshold to create binary image
   - Finds contours and checks if any exceed the minimum area
3. **API Notification**: When motion is detected, sends a POST request with timestamp and message
4. **Cooldown**: Prevents multiple API calls within the cooldown period

## Troubleshooting

- **Camera not found**: Make sure your webcam is connected and not in use by another application
- **OpenCV not found**: Ensure OpenCV is properly installed and GoCV can find it
- **API errors**: Check that the endpoint URL is correct and the server is running

## License

MIT
