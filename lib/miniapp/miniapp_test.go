package miniapp

import (
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

const token = "123:ABC"

var now = time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)

// signed builds initData the way Telegram does, for a user and an age.
func signed(t *testing.T, botToken string, userID int64, age time.Duration, extra ...string) string {
	t.Helper()
	vals := url.Values{
		"auth_date": {fmt.Sprint(now.Add(-age).Unix())},
		"user":      {fmt.Sprintf(`{"id":%d,"first_name":"A"}`, userID)},
		"query_id":  {"AAH"},
	}
	for i := 0; i+1 < len(extra); i += 2 {
		vals.Set(extra[i], extra[i+1])
	}
	secret := hmacSHA256([]byte("WebAppData"), []byte(botToken))
	vals.Set("hash", hex.EncodeToString(hmacSHA256(secret, []byte(dataCheckString(vals, "hash")))))
	return vals.Encode()
}

func TestValidate(t *testing.T) {
	good := signed(t, token, 42, time.Minute)
	if id, err := Validate(good, token, now, time.Hour); err != nil || id != 42 {
		t.Fatalf("good initData: %d, %v", id, err)
	}
	// a newer client adds a signature field the check may or may not cover
	withSig := signed(t, token, 42, time.Minute, "signature", "abc")
	if id, err := Validate(withSig, token, now, time.Hour); err != nil || id != 42 {
		t.Fatalf("with signature field: %d, %v", id, err)
	}

	tests := map[string]string{
		"wrong token":  signed(t, "999:XYZ", 42, time.Minute),
		"expired":      signed(t, token, 42, 2*time.Hour),
		"from future":  signed(t, token, 42, -time.Hour),
		"other user":   strings.Replace(good, "%22id%22%3A42", "%22id%22%3A43", 1),
		"no hash":      strings.Split(good, "&hash=")[0],
		"empty":        "",
		"garbage":      "%%%",
		"bad hash hex": "auth_date=1&hash=zz",
	}
	for name, data := range tests {
		if _, err := Validate(data, token, now, time.Hour); err == nil {
			t.Errorf("%s accepted", name)
		}
	}
}

func newServer(t *testing.T) *httptest.Server {
	t.Helper()
	s := New(Options{Token: token, Allowed: func(_ context.Context, id int64) bool { return id == 42 }, MaxAge: time.Hour, MaxViewers: 1, MaxStream: time.Minute,
		Events: func(w http.ResponseWriter, r *http.Request) { w.Write([]byte("data: [1,0.5]\n\n")) }})
	srv := httptest.NewServer(s.Handler())
	t.Cleanup(srv.Close)
	return srv
}

func get(t *testing.T, srv *httptest.Server, path string) *http.Response {
	t.Helper()
	resp, err := http.Get(srv.URL + path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { resp.Body.Close() })
	return resp
}

func TestRoutesNeedAuth(t *testing.T) {
	srv := newServer(t)
	// use the real clock: the handlers do
	ok := url.QueryEscape(signedNow(t, 42))
	other := url.QueryEscape(signedNow(t, 7))

	if r := get(t, srv, "/"); r.StatusCode != 200 || !strings.Contains(r.Header.Get("Content-Type"), "text/html") {
		t.Errorf("page: %d %s", r.StatusCode, r.Header.Get("Content-Type"))
	}
	for _, path := range []string{"/api/check", "/stream", "/events"} {
		if r := get(t, srv, path); r.StatusCode != 401 {
			t.Errorf("%s without auth: %d", path, r.StatusCode)
		}
		if r := get(t, srv, path+"?auth="+other); r.StatusCode != 401 {
			t.Errorf("%s for a user not allowed: %d", path, r.StatusCode)
		}
	}
	if r := get(t, srv, "/api/check?auth="+ok); r.StatusCode != 204 {
		t.Errorf("check: %d", r.StatusCode)
	}
	if r := get(t, srv, "/events?auth="+ok); r.StatusCode != 200 {
		t.Errorf("events: %d", r.StatusCode)
	}
}

func signedNow(t *testing.T, userID int64) string {
	t.Helper()
	saved := now
	defer func() { now = saved }()
	now = time.Now()
	return signed(t, token, userID, time.Second)
}

func TestStream(t *testing.T) {
	s := New(Options{Token: token, Allowed: func(_ context.Context, id int64) bool { return id == 42 }, MaxAge: time.Hour, MaxViewers: 1, MaxStream: time.Minute})
	srv := httptest.NewServer(s.Handler())
	defer srv.Close()
	auth := "?auth=" + url.QueryEscape(signedNow(t, 42))

	if s.Wanted() {
		t.Fatal("wanted with no viewer")
	}
	resp, err := http.Get(srv.URL + "/stream" + auth)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if ct := resp.Header.Get("Content-Type"); !strings.HasPrefix(ct, "multipart/x-mixed-replace") {
		t.Fatalf("content type %q", ct)
	}
	deadline := time.Now().Add(5 * time.Second)
	for !s.Wanted() && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if !s.Wanted() {
		t.Fatal("not wanted with a viewer")
	}

	// the second viewer is over the limit
	if r := get2(t, srv.URL+"/stream"+auth); r != 503 {
		t.Errorf("second viewer: %d", r)
	}

	s.PushFrame([]byte("JPEGDATA"))
	buf := make([]byte, 256)
	n, _ := resp.Body.Read(buf)
	got := string(buf[:n])
	if !strings.Contains(got, "--frame") || !strings.Contains(got, "Content-Length: 8") || !strings.Contains(got, "JPEGDATA") {
		t.Errorf("frame = %q", got)
	}
}

func get2(t *testing.T, u string) int {
	t.Helper()
	resp, err := http.Get(u)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	return resp.StatusCode
}

func TestPushFrameKeepsOnlyTheLatest(t *testing.T) {
	s := New(Options{})
	ch := make(chan []byte, 1)
	s.subs[ch] = struct{}{}
	s.PushFrame([]byte("a"))
	s.PushFrame([]byte("b"))
	if got := string(<-ch); got != "b" {
		t.Errorf("got %q, want the latest", got)
	}
}

func TestParseURL(t *testing.T) {
	for in, want := range map[string]string{"": "", "  ": "", "https://cam.tail1234.ts.net": "https://cam.tail1234.ts.net"} {
		if got, err := ParseURL(in); err != nil || got != want {
			t.Errorf("ParseURL(%q) = %q, %v", in, got, err)
		}
	}
	for _, in := range []string{"http://cam.ts.net", "cam.ts.net", "https://", "ftp://x"} {
		if _, err := ParseURL(in); err == nil {
			t.Errorf("ParseURL(%q) accepted", in)
		}
	}
}

type fakeMembers struct {
	in    map[int64]bool
	err   error
	calls int
}

func (f *fakeMembers) IsMember(_ context.Context, id int64) (bool, error) {
	f.calls++
	return f.in[id], f.err
}

func TestAccessPrivateChat(t *testing.T) {
	a, err := NewAccess("42", nil)
	if err != nil {
		t.Fatal(err)
	}
	if !a.Allowed(context.Background(), 42) || a.Allowed(context.Background(), 43) {
		t.Error("only the user of a private chat may get in")
	}
}

func TestAccessGroup(t *testing.T) {
	m := &fakeMembers{in: map[int64]bool{7: true}}
	a, err := NewAccess("-100123", m)
	if err != nil {
		t.Fatal(err)
	}
	clock := time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)
	a.now = func() time.Time { return clock }
	ctx := context.Background()

	if !a.Allowed(ctx, 7) || a.Allowed(ctx, 8) {
		t.Fatal("members in, others out")
	}
	a.Allowed(ctx, 7)
	a.Allowed(ctx, 8)
	if m.calls != 2 {
		t.Errorf("answers are not remembered: %d calls", m.calls)
	}

	// a no is forgotten soon: somebody who just joined gets in
	m.in[8] = true
	clock = clock.Add(noFor + time.Second)
	if !a.Allowed(ctx, 8) {
		t.Error("a new member is still refused after the no expired")
	}
	// a yes lasts longer, then somebody who left is out
	delete(m.in, 7)
	if !a.Allowed(ctx, 7) {
		t.Error("a yes was not kept")
	}
	clock = clock.Add(yesFor + time.Second)
	if a.Allowed(ctx, 7) {
		t.Error("somebody who left is still let in")
	}
}

func TestAccessGroupFailsClosed(t *testing.T) {
	m := &fakeMembers{in: map[int64]bool{7: true}, err: errors.New("down")}
	a, _ := NewAccess("-100123", m)
	clock := time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)
	a.now = func() time.Time { return clock }
	if a.Allowed(context.Background(), 7) {
		t.Fatal("let in with the check failing")
	}
	// the failure is not held against the user for long
	m.err = nil
	clock = clock.Add(errorFor + time.Second)
	if !a.Allowed(context.Background(), 7) {
		t.Error("still refused after the failure was forgotten")
	}
}

func TestNewAccess(t *testing.T) {
	for _, in := range []string{"", "abc", "0"} {
		if _, err := NewAccess(in, &fakeMembers{}); err == nil {
			t.Errorf("NewAccess(%q) accepted", in)
		}
	}
	if _, err := NewAccess("-100123", nil); err == nil {
		t.Error("a group with no way to check it accepted")
	}
}
