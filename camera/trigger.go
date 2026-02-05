package camera

import (
	"fmt"
	"image"
	"security-camera/telegram"
)

type TelegramTrigger struct {
	telegramBot *telegram.TelegramBot
}

func NewTelegramTrigger(telegramBot *telegram.TelegramBot) *TelegramTrigger {
	return &TelegramTrigger{telegramBot: telegramBot}
}

func (tt *TelegramTrigger) OnMovementDetected(img image.Image) {
	if tt.telegramBot != nil {
		fmt.Println("Movement detected! Sending alert via Telegram...")
		err := tt.telegramBot.SendAlert("Movement detected by the security camera!", telegram.SendMessageOptions{
			Image: &img,
		})
		if err != nil {
			fmt.Printf("Error sending Telegram alert: %v\n", err)
		}
	}
}
