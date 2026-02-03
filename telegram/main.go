package telegram

import (
	"bytes"
	"context"
	"fmt"
	"image"
	"image/jpeg"
	"io"
	"os"
	"os/signal"
	"security-camera/db"
	"security-camera/entities/user"

	"github.com/go-telegram/bot"
	"github.com/go-telegram/bot/models"
)

type TelegramBot struct {
	bot      *bot.Bot
	ctx      context.Context
	cancel   context.CancelFunc
	db       *db.DbStruct
	userRepo *user.UserRepository
}

func NewTelegramBot(db *db.DbStruct, apiKey string) (*TelegramBot, error) {
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)

	// Initialize user repo early to use in handler
	userRepo := user.NewUserRepository(db)

	opts := []bot.Option{
		bot.WithDefaultHandler(func(ctx context.Context, b *bot.Bot, update *models.Update) {
			handler(ctx, b, update, userRepo)
		}),
	}

	b, err := bot.New(apiKey, opts...)
	if err != nil {
		cancel()
		return nil, err
	}
	go b.Start(ctx)
	return &TelegramBot{bot: b, ctx: ctx, cancel: cancel, db: db, userRepo: userRepo}, nil
}

// Stop gracefully stops the bot
func (tb *TelegramBot) Stop() {
	if tb.cancel != nil {
		tb.cancel()
	}
}

func handler(ctx context.Context, b *bot.Bot, update *models.Update, userRepo *user.UserRepository) {
	user := user.User{
		TelegramID: fmt.Sprint(update.Message.From.ID),
		Username:   update.Message.From.Username,
		ChatID:     update.Message.Chat.ID,
		Admin:      false,
	}

	err := userRepo.Upsert(map[string]interface{}{"telegramid": user.TelegramID}, &user)
	if err != nil {
		fmt.Printf("Error upserting user: %v\n", err)
		return
	}

	b.SendMessage(ctx, &bot.SendMessageParams{
		ChatID: update.Message.Chat.ID,
		Text:   "User registered successfully!",
	})
}

type SendMessageOptions struct {
	Image image.Image
	Text  string
}

func imageToReader(img image.Image) (io.Reader, error) {
	buf := new(bytes.Buffer)
	err := jpeg.Encode(buf, img, nil)
	if err != nil {
		return nil, fmt.Errorf("failed to encode image: %w", err)
	}
	return buf, nil
}

func (tb *TelegramBot) SendAlert(text string) error {
	users, err := tb.userRepo.Find(map[string]interface{}{"admin": true})
	if err != nil {
		return fmt.Errorf("failed to retrieve admin users: %w", err)
	}

	for _, user := range users {
		if err != nil {
			fmt.Printf("Invalid chat ID for user %s: %v\n", user.Username, err)
			continue
		}

		err = tb.sendMessage(user.ChatID, SendMessageOptions{Text: text})
		if err != nil {
			fmt.Printf("Error sending alert to %s: %v\n", user.Username, err)
		}
	}
	return nil
}

func (tb *TelegramBot) sendMessage(chatID int64, options SendMessageOptions) error {
	_, err := tb.bot.SendMessage(tb.ctx, &bot.SendMessageParams{
		ChatID: chatID,
		Text:   options.Text,
	})
	if err != nil {
		return fmt.Errorf("failed to send message: %w", err)
	}

	if options.Image != nil {
		imageReader, err := imageToReader(options.Image)
		if err != nil {
			return fmt.Errorf("failed to convert image: %w", err)
		}

		photoParams := bot.SendPhotoParams{
			ChatID: chatID,
		}
		photoUpload := &models.InputFileUpload{
			Filename: "image.jpg",
			Data:     imageReader,
		}
		photoParams.Photo = photoUpload

		_, err = tb.bot.SendPhoto(tb.ctx, &photoParams)
		if err != nil {
			return fmt.Errorf("failed to send photo: %w", err)
		}
	}

	return err
}
