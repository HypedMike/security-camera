package miniapp

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/url"
	"sort"
	"strconv"
	"strings"
	"time"
)

// errUnauthorized is the only thing a caller learns about a failed check.
var errUnauthorized = errors.New("miniapp: unauthorized")

// Validate checks the initData string Telegram gives a mini app when it is
// opened, and returns the id of the Telegram user it was made for.
//
// Telegram signs initData with a key derived from the bot token, so only a
// real opening of the app from Telegram can produce one that passes. The
// signature is HMAC-SHA256 over the fields (except hash) sorted by name, as
// "key=value" lines. initData older than maxAge is refused, so a leaked one
// stops working.
func Validate(initData, botToken string, now time.Time, maxAge time.Duration) (int64, error) {
	vals, err := url.ParseQuery(initData)
	if err != nil {
		return 0, errUnauthorized
	}
	hash, err := hex.DecodeString(vals.Get("hash"))
	if err != nil || len(hash) != sha256.Size {
		return 0, errUnauthorized
	}

	secret := hmacSHA256([]byte("WebAppData"), []byte(botToken))
	// newer clients add a "signature" field meant for third-party checks;
	// whether the bot-token signature covers it is not something to guess
	// at, and accepting either is as safe: the HMAC still has to match
	ok := false
	for _, skip := range [][]string{{"hash"}, {"hash", "signature"}} {
		if hmac.Equal(hmacSHA256(secret, []byte(dataCheckString(vals, skip...))), hash) {
			ok = true
			break
		}
	}
	if !ok {
		return 0, errUnauthorized
	}

	sec, err := strconv.ParseInt(vals.Get("auth_date"), 10, 64)
	if err != nil {
		return 0, errUnauthorized
	}
	age := now.Sub(time.Unix(sec, 0))
	if age > maxAge || age < -5*time.Minute {
		return 0, errUnauthorized
	}

	var user struct {
		ID int64 `json:"id"`
	}
	if err := json.Unmarshal([]byte(vals.Get("user")), &user); err != nil || user.ID == 0 {
		return 0, errUnauthorized
	}
	return user.ID, nil
}

// dataCheckString joins the fields, but the skipped ones, sorted by name.
func dataCheckString(vals url.Values, skip ...string) string {
	var lines []string
fields:
	for k, v := range vals {
		for _, s := range skip {
			if k == s {
				continue fields
			}
		}
		lines = append(lines, k+"="+v[0])
	}
	sort.Strings(lines)
	return strings.Join(lines, "\n")
}

func hmacSHA256(key, data []byte) []byte {
	m := hmac.New(sha256.New, key)
	m.Write(data)
	return m.Sum(nil)
}
