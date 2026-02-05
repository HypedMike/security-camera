package camera

import (
	"fmt"
	"image"
	"security-camera/db"
	"security-camera/entities/notification"
	"time"

	"gocv.io/x/gocv"
)

type IWebcamTrigger interface {
	OnMovementDetected(image.Image)
}

type WebcamService struct {
	webcam *gocv.VideoCapture
	window *gocv.Window

	trigger           *IWebcamTrigger
	notificationLogic *notification.NotificationLogic

	framesChan  chan gocv.Mat
	showDisplay bool
}

func NewWebcamService(showDisplay bool, db *db.DbStruct) (*WebcamService, error) {
	webcam, err := gocv.OpenVideoCapture(0)
	if err != nil {
		return nil, err
	}

	var window *gocv.Window
	if showDisplay {
		window = gocv.NewWindow("Webcam")
	}

	return &WebcamService{
		webcam:            webcam,
		window:            window,
		framesChan:        make(chan gocv.Mat),
		showDisplay:       showDisplay,
		notificationLogic: notification.NewNotificationLogic(db),
	}, nil
}

func (ws *WebcamService) SetTrigger(trigger IWebcamTrigger) {
	ws.trigger = &trigger
}

func (ws *WebcamService) Close() {
	ws.webcam.Close()
	if ws.window != nil {
		ws.window.Close()
	}
	close(ws.framesChan)
}

// CalculateFrameSimilarity computes similarity between two frames
// Returns 1.0 for identical frames, 0.0 for completely different frames
func calculateFrameSimilarity(frame1, frame2 gocv.Mat) float64 {
	if frame1.Empty() || frame2.Empty() {
		return 0.0
	}

	// Ensure frames have same dimensions
	if frame1.Rows() != frame2.Rows() || frame1.Cols() != frame2.Cols() {
		return 0.0
	}

	// Calculate absolute difference
	diff := gocv.NewMat()
	defer diff.Close()
	gocv.AbsDiff(frame1, frame2, &diff)

	// Convert to grayscale for simpler calculation
	gray := gocv.NewMat()
	defer gray.Close()
	gocv.CvtColor(diff, &gray, gocv.ColorBGRToGray)

	// Calculate total possible difference (all pixels at max value)
	totalPixels := float64(gray.Rows() * gray.Cols())
	maxPossibleDiff := totalPixels * 255.0

	// Sum all pixel differences using Norm L1 (sum of absolute values)
	actualDiff := gocv.Norm(gray, gocv.NormL1)

	// Calculate similarity: 1.0 - (actualDiff / maxPossibleDiff)
	similarity := 1.0 - (actualDiff / maxPossibleDiff)

	return similarity
}

func checkArrayMovement(imgs []gocv.Mat) float64 {
	if len(imgs) < 2 {
		return 0.0
	}
	similarity := 0.0
	for i := 1; i < len(imgs); i++ {
		similarity += calculateFrameSimilarity(imgs[i-1], imgs[i])
	}
	averageSimilarity := similarity / float64(len(imgs)-1)
	return averageSimilarity
}

func (ws *WebcamService) createNotification() error {
	return ws.notificationLogic.CreateNotification(notification.CreateNotificationRequest{
		Message:   &[]string{"Motion detected"}[0],
		Timestamp: time.Now().Unix(),
	})
}

func (ws *WebcamService) ElaborateFrames(maxFrames int) {
	imgsArray := []gocv.Mat{}
	maxLength := 50
	counter := 0

	for frame := range ws.framesChan {
		counter++
		if len(imgsArray) >= maxLength {
			imgsArray = imgsArray[1:]
		}
		imgsArray = append(imgsArray, frame)

		// Only show display if enabled (must be on main thread on macOS)
		if ws.showDisplay && ws.window != nil {
			ws.window.IMShow(frame)
			ws.window.WaitKey(1)
		}

		if counter >= maxFrames {
			similarity := checkArrayMovement(imgsArray)
			fmt.Printf("Frame similarity: %.4f\n", similarity)
			// err := ws.createNotification()
			// if err != nil {
			// 	fmt.Printf("Error creating notification: %v\n", err)
			// }
			if similarity*10-9 < 0.99 && ws.trigger != nil {
				// Convert gocv.Mat to image.Image
				img, err := frame.ToImage()
				if err != nil {
					fmt.Printf("Error converting frame to image: %v\n", err)
				} else {
					(*ws.trigger).OnMovementDetected(img)
				}
			}
			counter = 0
		}
	}
}

func (ws *WebcamService) ListenToFrames() error {
	img := gocv.NewMat()
	defer img.Close()

	for {
		if ok := ws.webcam.Read(&img); !ok {
			return fmt.Errorf("cannot read from webcam")
		}
		if img.Empty() {
			continue
		}
		ws.framesChan <- img.Clone()
	}
}
