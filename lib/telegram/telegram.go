package telegram

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime/multipart"
	"net/http"
	"time"
)

const defaultAPI = "https://api.telegram.org"

// Bot sends messages to a Telegram chat through the Bot API.
type Bot struct {
	token  string
	chatID string
	api    string
	client *http.Client
}

func New(token, chatID string) (*Bot, error) {
	if token == "" || chatID == "" {
		return nil, errors.New("telegram: token and chat id are required")
	}
	return &Bot{
		token:  token,
		chatID: chatID,
		api:    defaultAPI,
		client: &http.Client{Timeout: 10 * time.Second},
	}, nil
}

// SendMessage posts text to the configured chat.
func (b *Bot) SendMessage(text string) error {
	body, err := json.Marshal(map[string]string{"chat_id": b.chatID, "text": text})
	if err != nil {
		return err
	}

	resp, err := b.client.Post(fmt.Sprintf("%s/bot%s/sendMessage", b.api, b.token), "application/json", bytes.NewReader(body))
	if err != nil {
		// the url contains the token, so don't leak it through the error
		return errors.New("telegram: request failed")
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		msg, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return fmt.Errorf("telegram: status %d: %s", resp.StatusCode, msg)
	}
	return nil
}

// SendAnimation uploads a GIF to the configured chat with an optional caption.
func (b *Bot) SendAnimation(gif []byte, caption string) error {
	var body bytes.Buffer
	w := multipart.NewWriter(&body)
	w.WriteField("chat_id", b.chatID)
	if caption != "" {
		w.WriteField("caption", caption)
	}
	part, err := w.CreateFormFile("animation", "alarm.gif")
	if err != nil {
		return err
	}
	if _, err := part.Write(gif); err != nil {
		return err
	}
	if err := w.Close(); err != nil {
		return err
	}

	resp, err := b.client.Post(fmt.Sprintf("%s/bot%s/sendAnimation", b.api, b.token), w.FormDataContentType(), &body)
	if err != nil {
		return errors.New("telegram: request failed")
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		msg, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return fmt.Errorf("telegram: status %d: %s", resp.StatusCode, msg)
	}
	return nil
}
