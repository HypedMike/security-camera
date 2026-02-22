package camera

import (
	"bytes"
	"errors"
	"fmt"
	"image"
	"image/color"
	"image/jpeg"
	"math"
	"security-camera/db"
	"security-camera/entities/notification"
	"time"

	"github.com/blackjack/webcam"
)

// V4L2 pixel format codes
const (
	v4l2PixFmtMJPEG = webcam.PixelFormat(0x47504a4d) // MJPG
	v4l2PixFmtYUYV  = webcam.PixelFormat(0x56595559) // YUYV
)

type IWebcamTrigger interface {
	OnMovementDetected(image.Image)
}

type WebcamService struct {
	webcam      *webcam.Webcam
	pixelFormat webcam.PixelFormat
	width       uint32
	height      uint32

	trigger           *IWebcamTrigger
	notificationLogic *notification.NotificationLogic

	framesChan  chan image.Image
	showDisplay bool
}

func NewWebcamService(showDisplay bool, db *db.DbStruct) (*WebcamService, error) {
	cam, err := webcam.Open("/dev/video0")
	if err != nil {
		return nil, err
	}

	// Prefer MJPEG; fall back to YUYV
	formats := cam.GetSupportedFormats()
	var selectedFormat webcam.PixelFormat
	for f := range formats {
		if f == v4l2PixFmtMJPEG {
			selectedFormat = f
			break
		}
	}
	if selectedFormat == 0 {
		for f := range formats {
			if f == v4l2PixFmtYUYV {
				selectedFormat = f
				break
			}
		}
	}
	if selectedFormat == 0 {
		cam.Close()
		return nil, fmt.Errorf("no supported pixel format found (need MJPEG or YUYV)")
	}

	// Select the largest supported resolution
	sizes := cam.GetSupportedFrameSizes(selectedFormat)
	if len(sizes) == 0 {
		cam.Close()
		return nil, fmt.Errorf("no supported frame sizes found")
	}
	selected := sizes[0]
	for _, s := range sizes[1:] {
		if s.MaxWidth*s.MaxHeight > selected.MaxWidth*selected.MaxHeight {
			selected = s
		}
	}

	_, width, height, err := cam.SetImageFormat(selectedFormat, selected.MaxWidth, selected.MaxHeight)
	if err != nil {
		cam.Close()
		return nil, fmt.Errorf("failed to set image format: %w", err)
	}

	return &WebcamService{
		webcam:            cam,
		pixelFormat:       selectedFormat,
		width:             width,
		height:            height,
		framesChan:        make(chan image.Image),
		showDisplay:       showDisplay,
		notificationLogic: notification.NewNotificationLogic(db),
	}, nil
}

func (ws *WebcamService) SetTrigger(trigger IWebcamTrigger) {
	ws.trigger = &trigger
}

func (ws *WebcamService) Close() {
	ws.webcam.StopStreaming()
	ws.webcam.Close()
	close(ws.framesChan)
}

// calculateFrameSimilarity computes similarity between two frames.
// Returns 1.0 for identical frames, 0.0 for completely different frames.
func calculateFrameSimilarity(frame1, frame2 image.Image) float64 {
	b1 := frame1.Bounds()
	b2 := frame2.Bounds()
	if b1 != b2 {
		return 0.0
	}

	totalPixels := float64(b1.Dx() * b1.Dy())
	if totalPixels == 0 {
		return 0.0
	}
	maxPossibleDiff := totalPixels * 255.0

	var actualDiff float64
	for y := b1.Min.Y; y < b1.Max.Y; y++ {
		for x := b1.Min.X; x < b1.Max.X; x++ {
			r1, g1, b1c, _ := frame1.At(x, y).RGBA()
			r2, g2, b2c, _ := frame2.At(x, y).RGBA()
			// ITU-R BT.601 luma coefficients: convert 16-bit RGBA to 8-bit grayscale
			gray1 := 0.299*float64(r1>>8) + 0.587*float64(g1>>8) + 0.114*float64(b1c>>8)
			gray2 := 0.299*float64(r2>>8) + 0.587*float64(g2>>8) + 0.114*float64(b2c>>8)
			actualDiff += math.Abs(gray1 - gray2)
		}
	}

	return 1.0 - (actualDiff / maxPossibleDiff)
}

func checkArrayMovement(imgs []image.Image) float64 {
	if len(imgs) < 2 {
		return 0.0
	}
	similarity := 0.0
	for i := 1; i < len(imgs); i++ {
		similarity += calculateFrameSimilarity(imgs[i-1], imgs[i])
	}
	return similarity / float64(len(imgs)-1)
}

func (ws *WebcamService) createNotification() error {
	return ws.notificationLogic.CreateNotification(notification.CreateNotificationRequest{
		Message:   &[]string{"Motion detected"}[0],
		Timestamp: time.Now().Unix(),
	})
}

func (ws *WebcamService) ElaborateFrames(maxFrames int) {
	imgsArray := []image.Image{}
	maxLength := 50
	counter := 0

	for frame := range ws.framesChan {
		counter++
		if len(imgsArray) >= maxLength {
			imgsArray = imgsArray[1:]
		}
		imgsArray = append(imgsArray, frame)

		if counter >= maxFrames {
			similarity := checkArrayMovement(imgsArray)
			fmt.Printf("Frame similarity: %.4f\n", similarity)
			// err := ws.createNotification()
			// if err != nil {
			// 	fmt.Printf("Error creating notification: %v\n", err)
			// }
			if similarity*10-9 < 0.99 && ws.trigger != nil {
				(*ws.trigger).OnMovementDetected(frame)
			}
			counter = 0
		}
	}
}

// decodeFrame converts raw V4L2 frame bytes to image.Image.
func (ws *WebcamService) decodeFrame(data []byte) (image.Image, error) {
	switch ws.pixelFormat {
	case v4l2PixFmtMJPEG:
		return jpeg.Decode(bytes.NewReader(data))
	case v4l2PixFmtYUYV:
		return yuyvToImage(data, int(ws.width), int(ws.height))
	default:
		return nil, fmt.Errorf("unsupported pixel format: %d", ws.pixelFormat)
	}
}

// yuyvToImage converts a YUYV (YCbCr 4:2:2) byte slice to an RGBA image.
func yuyvToImage(data []byte, width, height int) (image.Image, error) {
	expected := width * height * 2
	if len(data) < expected {
		return nil, fmt.Errorf("YUYV data too short: got %d, want %d", len(data), expected)
	}
	img := image.NewRGBA(image.Rect(0, 0, width, height))
	for y := 0; y < height; y++ {
		for x := 0; x < width; x += 2 {
			offset := (y*width + x) * 2
			y0 := int(data[offset])
			u := int(data[offset+1])
			y1 := int(data[offset+2])
			v := int(data[offset+3])
			img.SetRGBA(x, y, yuvToRGBA(y0, u, v))
			if x+1 < width {
				img.SetRGBA(x+1, y, yuvToRGBA(y1, u, v))
			}
		}
	}
	return img, nil
}

// yuvToRGBA converts a single YUV pixel to RGBA.
func yuvToRGBA(y, u, v int) color.RGBA {
	c := y - 16
	d := u - 128
	e := v - 128
	r := clamp((298*c + 409*e + 128) >> 8)
	g := clamp((298*c - 100*d - 208*e + 128) >> 8)
	b := clamp((298*c + 516*d + 128) >> 8)
	return color.RGBA{R: uint8(r), G: uint8(g), B: uint8(b), A: 255}
}

func clamp(v int) int {
	if v < 0 {
		return 0
	}
	if v > 255 {
		return 255
	}
	return v
}

func (ws *WebcamService) ListenToFrames() error {
	if err := ws.webcam.StartStreaming(); err != nil {
		return fmt.Errorf("failed to start streaming: %w", err)
	}

	for {
		if err := ws.webcam.WaitForFrame(5); err != nil {
			var timeout *webcam.Timeout
			if errors.As(err, &timeout) {
				continue
			}
			return fmt.Errorf("error waiting for frame: %w", err)
		}

		data, err := ws.webcam.ReadFrame()
		if err != nil {
			return fmt.Errorf("error reading frame: %w", err)
		}
		if len(data) == 0 {
			continue
		}

		img, err := ws.decodeFrame(data)
		if err != nil {
			fmt.Printf("Error decoding frame: %v\n", err)
			continue
		}

		ws.framesChan <- img
	}
}
