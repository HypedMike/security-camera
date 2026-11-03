package miniapp

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
)

// Members tells whether a Telegram user is in the configured chat.
type Members interface {
	IsMember(ctx context.Context, userID int64) (bool, error)
}

// How long an answer is remembered. Asking Telegram on every request would be
// slow and would let anyone who opens the app make the program call Telegram
// as often as they like. A yes lasts longer than a no, so somebody who has
// just joined the group gets in soon, and somebody who left is out in minutes.
// A failed check is only remembered briefly: it says nothing about the user.
const (
	yesFor   = 10 * time.Minute
	noFor    = time.Minute
	errorFor = 10 * time.Second

	// checkTimeout is how long one question to Telegram may take.
	checkTimeout = 5 * time.Second

	// the cache is cleaned when it has this many answers
	cacheLimit = 1000
)

// Access decides which Telegram users may open the mini app: those who are in
// the chat the bot is configured for (TELEGRAM_CHAT_ID).
//
//   - private chat: only its user. A private chat's id is the id of the user.
//   - group: whoever is in the group, asked to Telegram when they open the app
//     (Telegram can't list a group's members) and remembered for a while.
type Access struct {
	owner   int64 // the user of a private chat, 0 for a group
	members Members
	now     func() time.Time

	mu    sync.Mutex
	cache map[int64]verdict
}

type verdict struct {
	ok    bool
	until time.Time
}

// NewAccess returns the Access for the chat id. members is only used when the
// chat is a group.
func NewAccess(chatID string, members Members) (*Access, error) {
	id, err := strconv.ParseInt(strings.TrimSpace(chatID), 10, 64)
	if err != nil || id == 0 {
		return nil, errors.New("TELEGRAM_CHAT_ID is not a chat id")
	}
	a := &Access{now: time.Now, cache: map[int64]verdict{}}
	if id > 0 {
		a.owner = id
		return a, nil
	}
	if members == nil {
		return nil, errors.New("a group chat needs a way to check who is in it")
	}
	a.members = members
	return a, nil
}

// Allowed reports whether the Telegram user may open the mini app.
func (a *Access) Allowed(ctx context.Context, userID int64) bool {
	if a.owner != 0 {
		return userID == a.owner
	}

	now := a.now()
	a.mu.Lock()
	v, ok := a.cache[userID]
	a.mu.Unlock()
	if ok && now.Before(v.until) {
		return v.ok
	}

	ctx, cancel := context.WithTimeout(ctx, checkTimeout)
	defer cancel()
	member, err := a.members.IsMember(ctx, userID)
	v = verdict{ok: member, until: now.Add(noFor)}
	switch {
	case err != nil:
		// closed: when in doubt, nobody gets the camera
		fmt.Fprintf(os.Stderr, "mini app: can't check whether user %d is in the group: %v\n", userID, err)
		v = verdict{until: now.Add(errorFor)}
	case member:
		v.until = now.Add(yesFor)
	}

	a.mu.Lock()
	defer a.mu.Unlock()
	if len(a.cache) >= cacheLimit {
		for id, old := range a.cache {
			if !now.Before(old.until) {
				delete(a.cache, id)
			}
		}
	}
	a.cache[userID] = v
	return v.ok
}
