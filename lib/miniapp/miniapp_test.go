package miniapp

import (
	"encoding/hex"
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
	s := New(Options{Token: token, Users: []int64{42}, MaxAge: time.Hour, MaxViewers: 1, MaxStream: time.Minute,
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
	s := New(Options{Token: token, Users: []int64{42}, MaxAge: time.Hour, MaxViewers: 1, MaxStream: time.Minute})
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

func TestParseUsers(t *testing.T) {
	if got, err := ParseUsers("42", ""); err != nil || len(got) != 1 || got[0] != 42 {
		t.Errorf("private chat: %v, %v", got, err)
	}
	if got, err := ParseUsers("42", " 7, 8 ,"); err != nil || len(got) != 3 || got[2] != 8 {
		t.Errorf("extra users: %v, %v", got, err)
	}
	if got, err := ParseUsers("-100123", "7"); err != nil || len(got) != 1 || got[0] != 7 {
		t.Errorf("group chat: %v, %v", got, err)
	}
	if _, err := ParseUsers("-100123", ""); err == nil {
		t.Error("group chat with nobody allowed accepted")
	}
	if _, err := ParseUsers("42", "abc"); err == nil {
		t.Error("bad id accepted")
	}
	if _, err := ParseUsers("42", "-5"); err == nil {
		t.Error("negative id accepted")
	}
}

func TestSettings(t *testing.T) {
	if u, users, err := Settings("", "-100", ""); u != "" || users != nil || err != nil {
		t.Errorf("off: %q %v %v", u, users, err)
	}
	if u, users, err := Settings("https://x.ts.net", "42", "7"); err != nil || u != "https://x.ts.net" || len(users) != 2 {
		t.Errorf("on: %q %v %v", u, users, err)
	}
	// each mistake is an error that names its variable, with the mini app left off
	u, _, err := Settings("https://x.ts.net", "-100", "")
	if err == nil || u != "" || !strings.Contains(err.Error(), "MINI_APP_USER_IDS") {
		t.Errorf("group without users: %q %v", u, err)
	}
	if _, _, err := Settings("http://x", "42", ""); err == nil || !strings.Contains(err.Error(), "MINI_APP_URL") {
		t.Errorf("http address: %v", err)
	}
}
