package telegram

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
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
