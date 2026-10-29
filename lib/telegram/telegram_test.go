package telegram

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
)

func TestSendMessage(t *testing.T) {
	var gotPath string
	var got map[string]string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		json.NewDecoder(r.Body).Decode(&got)
	}))
	defer srv.Close()

	b, _ := New("TOKEN", "42")
	b.api = srv.URL
	if err := b.SendMessage("hi"); err != nil {
		t.Fatal(err)
	}
	if gotPath != "/botTOKEN/sendMessage" || got["chat_id"] != "42" || got["text"] != "hi" {
		t.Errorf("unexpected request: %s %v", gotPath, got)
	}

	if _, err := New("", "42"); err == nil {
		t.Error("expected error for empty token")
	}
}

func TestSendAnimation(t *testing.T) {
	var gotPath, chat, caption string
	var file []byte
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		if err := r.ParseMultipartForm(1 << 20); err != nil {
			t.Error(err)
			return
		}
		chat, caption = r.FormValue("chat_id"), r.FormValue("caption")
		f, _, err := r.FormFile("animation")
		if err != nil {
			t.Error(err)
			return
		}
		file, _ = io.ReadAll(f)
	}))
	defer srv.Close()

	b, _ := New("TOKEN", "-42")
	b.api = srv.URL
	if err := b.SendAnimation([]byte("GIF89a"), "cap"); err != nil {
		t.Fatal(err)
	}
	if gotPath != "/botTOKEN/sendAnimation" || chat != "-42" || caption != "cap" || string(file) != "GIF89a" {
		t.Errorf("unexpected request: %s %q %q %q", gotPath, chat, caption, file)
	}
}

func TestListeners(t *testing.T) {
	var calls atomic.Int32
	var offsets []string
	done := make(chan struct{})
	var got []Message
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/botTOKEN/getUpdates" {
			t.Errorf("unexpected path %s", r.URL.Path)
		}
		offsets = append(offsets, r.URL.Query().Get("offset"))
		if calls.Add(1) == 1 {
			io.WriteString(w, `{"ok":true,"result":[
				{"update_id":10,"message":{"message_id":1,"date":100,"text":"hello","chat":{"id":-42},"from":{"username":"bob"}}},
				{"update_id":11,"message":{"message_id":2,"date":101,"text":"other chat","chat":{"id":7},"from":{"first_name":"Eve"}}},
				{"update_id":12,"message":{"message_id":3,"date":102,"chat":{"id":-42},"from":{"first_name":"Al"}}}]}`)
			return
		}
		<-done
		io.WriteString(w, `{"ok":true,"result":[]}`)
	}))
	defer srv.Close()

	b, _ := New("TOKEN", "-42")
	b.api = srv.URL
	ctx, cancel := context.WithCancel(context.Background())
	b.AddListener(func(m Message) {
		got = append(got, m)
		cancel()
		close(done)
	})
	if err := b.Run(ctx); err != context.Canceled {
		t.Fatalf("Run = %v", err)
	}
	if len(got) != 1 || got[0].Text != "hello" || got[0].From != "bob" || got[0].ID != 1 {
		t.Errorf("unexpected messages: %+v", got)
	}
	if offsets[0] != "0" {
		t.Errorf("first offset = %s", offsets[0])
	}
}

func TestSendPhotoAndCommand(t *testing.T) {
	var gotPath string
	var file []byte
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		r.ParseMultipartForm(1 << 20)
		f, _, err := r.FormFile("photo")
		if err != nil {
			t.Error(err)
			return
		}
		file, _ = io.ReadAll(f)
	}))
	defer srv.Close()

	b, _ := New("TOKEN", "1")
	b.api = srv.URL
	if err := b.SendPhoto([]byte("JPG"), ""); err != nil {
		t.Fatal(err)
	}
	if gotPath != "/botTOKEN/sendPhoto" || string(file) != "JPG" {
		t.Errorf("unexpected request: %s %q", gotPath, file)
	}

	for text, want := range map[string]string{"/snap": "snap", "/snap@mybot now": "snap", "hello": "", "/": ""} {
		if got := (Message{Text: text}).Command(); got != want {
			t.Errorf("Command(%q) = %q, want %q", text, got, want)
		}
	}

	for text, want := range map[string]string{"/threshold 0.1": "0.1", "/threshold@mybot 0.1 x": "0.1 x", "/snap": "", "hello there": ""} {
		if got := strings.Join((Message{Text: text}).Args(), " "); got != want {
			t.Errorf("Args(%q) = %q, want %q", text, got, want)
		}
	}
}
