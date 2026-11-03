package miniapp

import (
	"errors"
	"fmt"
	"net/url"
	"strconv"
	"strings"
)

// ParseURL checks the public address of the mini app (MINI_APP_URL). Telegram
// only opens https addresses. "" means the mini app is off.
func ParseURL(raw string) (string, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return "", nil
	}
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "https" || u.Host == "" {
		return "", fmt.Errorf("%q is not an https address (Telegram only opens https mini apps)", raw)
	}
	return u.String(), nil
}

// ParseUsers returns the Telegram user ids allowed to open the mini app: the
// owner of the chat (the chat id of a private chat is the user's id) and the
// ids in extra, separated by commas. A group chat has a negative id that is
// not a user, so then extra has to say who is allowed.
func ParseUsers(chatID, extra string) ([]int64, error) {
	var users []int64
	if id, err := strconv.ParseInt(strings.TrimSpace(chatID), 10, 64); err == nil && id > 0 {
		users = append(users, id)
	}
	for _, s := range strings.Split(extra, ",") {
		if s = strings.TrimSpace(s); s == "" {
			continue
		}
		id, err := strconv.ParseInt(s, 10, 64)
		if err != nil || id <= 0 {
			return nil, fmt.Errorf("%q is not a Telegram user id", s)
		}
		users = append(users, id)
	}
	if len(users) == 0 {
		return nil, errors.New("TELEGRAM_CHAT_ID is not a private chat, so MINI_APP_USER_IDS must list who may open the mini app")
	}
	return users, nil
}
