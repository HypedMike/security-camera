package camera

import "security-camera/db"

type CameraService struct {
	db      *db.DbStruct
	trigger IWebcamTrigger
	webcam  *WebcamService
}

func NewCameraService(db *db.DbStruct, trigger IWebcamTrigger) *CameraService {
	return &CameraService{
		db:      db,
		trigger: trigger,
	}
}

func (cs *CameraService) StartWebcamService() error {
	// Set showDisplay to false for headless server operation
	// Set to true only if running with GUI support on main thread
	webcamService, err := NewWebcamService(false)
	if err != nil {
		return err
	}

	webcamService.SetTrigger(cs.trigger)

	go webcamService.ElaborateFrames(100)

	cs.webcam = webcamService
	return cs.webcam.ListenToFrames()
}

func (cs *CameraService) Close() {
	if cs.webcam != nil {
		cs.webcam.Close()
	}
}
