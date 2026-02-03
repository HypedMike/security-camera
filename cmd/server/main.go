package main

import (
	"os"
	"security-camera/camera"
	"security-camera/db"
	"security-camera/telegram"

	"github.com/joho/godotenv"
)

func main() {
	godotenv.Load()

	telegramApiKey := os.Getenv("TELEGRAM_API_KEY")

	uri := os.Getenv("MONGODB_URI")
	dbName := os.Getenv("DB_NAME")
	database := db.NewDb(uri, dbName)

	telegramBot, err := telegram.NewTelegramBot(database, telegramApiKey)
	if err != nil {
		panic(err)
	}

	cameraService := camera.NewCameraService(database, camera.NewTelegramTrigger(telegramBot))
	err = cameraService.StartWebcamService()
	if err != nil {
		panic(err)
	}
	defer cameraService.Close()
	defer telegramBot.Stop()
}
