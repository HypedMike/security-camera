package main

import (
	"security-camera/camera"
	"security-camera/db"
	"security-camera/telegram"

	"github.com/joho/godotenv"
)

func main() {
	godotenv.Load()

	uri := "mongodb://localhost:27017/security-camera"
	database := db.NewDb(uri, "security-camera")

	telegramBot, err := telegram.NewTelegramBot(database)
	if err != nil {
		panic(err)
	}

	cameraService := camera.NewCameraService(database, camera.NewTelegramTrigger(telegramBot))
	err = cameraService.StartWebcamService()
	if err != nil {
		panic(err)
	}
	defer cameraService.Close()
}
