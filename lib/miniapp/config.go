package miniapp

import (
	"fmt"
	"net/url"
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
