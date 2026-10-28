package telegram

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime/multipart"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
)

const defaultAPI = "https://api.telegram.org"

// Bot sends messages to a Telegram chat through the Bot API.
type Bot struct {
	token  string
	chatID string
	api    string
	client *http.Client

	// long-polling needs a client whose timeout exceeds pollTimeout
	pollClient  *http.Client
	pollTimeout time.Duration

	mu        sync.RWMutex
	listeners []Listener
}

// Message is a text message received in the configured chat.
type Message struct {
	ID   int
	Text string
	// From is the sender's username, or their first name if they have none.
	From string
	Time time.Time
}

// Command returns the command in a message like "/snap" or "/snap@mybot arg"
// without the slash, or "" if the message is not a command.
func (m Message) Command() string {
	if !strings.HasPrefix(m.Text, "/") {
		return ""
	}
	word, _, _ := strings.Cut(strings.Fields(m.Text)[0], "@")
	return word[1:]
}

// Listener is called for every message received in the configured chat.
type Listener func(Message)

func New(token, chatID string) (*Bot, error) {
	if token == "" || chatID == "" {
		return nil, errors.New("telegram: token and chat id are required")
	}
	return &Bot{
		token:  token,
		chatID: chatID,
		api:    defaultAPI,
		client: &http.Client{Timeout: 10 * time.Second},

		pollClient:  &http.Client{Timeout: 40 * time.Second},
		pollTimeout: 30 * time.Second,
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
	return b.upload("sendAnimation", "animation", "alarm.gif", gif, caption)
}

// SendPhoto uploads a JPEG to the configured chat with an optional caption.
func (b *Bot) SendPhoto(jpeg []byte, caption string) error {
	return b.upload("sendPhoto", "photo", "snap.jpg", jpeg, caption)
}

func (b *Bot) upload(method, field, filename string, data []byte, caption string) error {
	var body bytes.Buffer
	w := multipart.NewWriter(&body)
	w.WriteField("chat_id", b.chatID)
	if caption != "" {
		w.WriteField("caption", caption)
	}
	part, err := w.CreateFormFile(field, filename)
	if err != nil {
		return err
	}
	if _, err := part.Write(data); err != nil {
		return err
	}
	if err := w.Close(); err != nil {
		return err
	}

	resp, err := b.client.Post(fmt.Sprintf("%s/bot%s/%s", b.api, b.token, method), w.FormDataContentType(), &body)
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

// AddListener registers a listener for messages sent to the configured chat.
// Messages from other chats are ignored. It is safe to call while Run is active.
func (b *Bot) AddListener(l Listener) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.listeners = append(b.listeners, l)
}

type update struct {
	UpdateID int `json:"update_id"`
	Message  *struct {
		MessageID int    `json:"message_id"`
		Date      int64  `json:"date"`
		Text      string `json:"text"`
		Chat      struct {
			ID int64 `json:"id"`
		} `json:"chat"`
		From struct {
			Username  string `json:"username"`
			FirstName string `json:"first_name"`
		} `json:"from"`
	} `json:"message"`
}

// Run long-polls for new messages and dispatches the ones from the configured
// chat to the listeners, in registration order, until ctx is cancelled.
// Listeners run synchronously, so a slow one delays the following messages.
// Transient request failures are retried after a short pause.
func (b *Bot) Run(ctx context.Context) error {
	offset := 0
	for ctx.Err() == nil {
		updates, err := b.getUpdates(ctx, offset)
		if err != nil {
			if ctx.Err() != nil {
				break
			}
			fmt.Fprintln(os.Stderr, err)
			select {
			case <-ctx.Done():
			case <-time.After(5 * time.Second):
			}
			continue
		}
		for _, u := range updates {
			offset = u.UpdateID + 1
			m := u.Message
			if m == nil || m.Text == "" || strconv.FormatInt(m.Chat.ID, 10) != b.chatID {
				continue
			}
			from := m.From.Username
			if from == "" {
				from = m.From.FirstName
			}
			b.dispatch(Message{ID: m.MessageID, Text: m.Text, From: from, Time: time.Unix(m.Date, 0)})
		}
	}
	return ctx.Err()
}

func (b *Bot) dispatch(m Message) {
	b.mu.RLock()
	listeners := b.listeners
	b.mu.RUnlock()
	for _, l := range listeners {
		l(m)
	}
}

func (b *Bot) getUpdates(ctx context.Context, offset int) ([]update, error) {
	url := fmt.Sprintf("%s/bot%s/getUpdates?offset=%d&timeout=%d&allowed_updates=%%5B%%22message%%22%%5D",
		b.api, b.token, offset, int(b.pollTimeout.Seconds()))
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, errors.New("telegram: request failed")
	}
	resp, err := b.pollClient.Do(req)
	if err != nil {
		// the url contains the token, so don't leak it through the error
		return nil, errors.New("telegram: request failed")
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		msg, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return nil, fmt.Errorf("telegram: status %d: %s", resp.StatusCode, msg)
	}
	var out struct {
		Result []update `json:"result"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		return nil, fmt.Errorf("telegram: decode updates: %w", err)
	}
	return out.Result, nil
}
