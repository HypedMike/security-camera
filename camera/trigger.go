package camera

import (
	"fmt"
	"security-camera/telegram"
)

type TelegramTrigger struct {
	telegramBot *telegram.TelegramBot
}

func NewTelegramTrigger(telegramBot *telegram.TelegramBot) *TelegramTrigger {
	return &TelegramTrigger{telegramBot: telegramBot}
}

func (tt *TelegramTrigger) OnMovementDetected() {
	if tt.telegramBot != nil {
		fmt.Println("Movement detected! Sending alert via Telegram...")
		err := tt.telegramBot.SendAlert("Movement detected by the security camera!")
		if err != nil {
			fmt.Printf("Error sending Telegram alert: %v\n", err)
		}
	}
}
