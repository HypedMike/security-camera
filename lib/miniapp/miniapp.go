// Package miniapp serves a Telegram mini app: a web page that shows the live
// camera view and the live motion graph inside Telegram.
//
// The server only listens on the local machine; a tunnel (Tailscale Funnel)
// makes it reachable over HTTPS. Every request except the page itself has to
// carry the initData Telegram gives the app when it opens, which proves the
// request comes from a Telegram user, and only the users in Options.Users
// are let in.
package miniapp

import (
	"embed"
	"fmt"
	"io/fs"
	"net/http"
	"slices"
	"sync"
	"sync/atomic"
	"time"
)

// dist is the React app (miniapp-web), built with npm run build and committed
// so the machine that runs the camera does not need Node.
//
//go:embed all:dist
var dist embed.FS

// Options configures a Server.
type Options struct {
	// Token is the bot token the initData signature is checked with.
	Token string
	// Users are the Telegram user ids allowed in.
	Users []int64
	// MaxAge is how old the initData may be.
	MaxAge time.Duration
	// MaxViewers is how many video streams may be open at once.
	MaxViewers int
	// MaxStream is the longest a single video stream is kept open, so a
	// forgotten app does not stream forever.
	MaxStream time.Duration
	// Events serves the motion graph as server-sent events.
	Events http.HandlerFunc
}

// Server is the HTTP side of the mini app.
type Server struct {
	opts    Options
	viewers atomic.Int32

	mu   sync.Mutex
	subs map[chan []byte]struct{}
}

// New returns a Server.
func New(opts Options) *Server {
	return &Server{opts: opts, subs: map[chan []byte]struct{}{}}
}

// Wanted reports whether anybody is watching, so the camera loop can skip
// encoding frames nobody will see.
func (s *Server) Wanted() bool { return s.viewers.Load() > 0 }

// PushFrame hands a JPEG to every open video stream. It never blocks: a slow
// viewer just skips frames. The server keeps the slice, so do not reuse it.
func (s *Server) PushFrame(jpeg []byte) {
	s.mu.Lock()
	defer s.mu.Unlock()
	for ch := range s.subs {
		select {
		case ch <- jpeg:
		default:
			// drop the stale frame the viewer has not taken yet
			select {
			case <-ch:
			default:
			}
			select {
			case ch <- jpeg:
			default:
			}
		}
	}
}

// ListenAndServe serves on addr (e.g. "127.0.0.1:8081"). It blocks.
func (s *Server) ListenAndServe(addr string) error {
	srv := &http.Server{Addr: addr, Handler: s.Handler(), ReadHeaderTimeout: 10 * time.Second}
	return srv.ListenAndServe()
}

// Handler returns the routes of the mini app.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	app, err := fs.Sub(dist, "dist")
	if err != nil {
		panic(err) // dist is embedded at build time
	}
	files := http.FileServerFS(app)
	// the page and its scripts are not secret: the data is what is guarded
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-cache")
		files.ServeHTTP(w, r)
	})
	mux.HandleFunc("/api/check", s.guard(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))
	mux.HandleFunc("/stream", s.guard(s.stream))
	if s.opts.Events != nil {
		mux.HandleFunc("/events", s.guard(s.opts.Events))
	}
	return mux
}

// guard lets only the allowed Telegram users through.
func (s *Server) guard(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		id, err := Validate(r.URL.Query().Get("auth"), s.opts.Token, time.Now(), s.opts.MaxAge)
		if err != nil || !slices.Contains(s.opts.Users, id) {
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		w.Header().Set("Cache-Control", "no-store")
		next(w, r)
	}
}

// stream sends the camera as MJPEG: a never-ending multipart response whose
// parts are JPEGs, which a plain <img> shows as video.
func (s *Server) stream(w http.ResponseWriter, r *http.Request) {
	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "streaming unsupported", http.StatusInternalServerError)
		return
	}
	if int(s.viewers.Add(1)) > s.opts.MaxViewers {
		s.viewers.Add(-1)
		http.Error(w, "too many viewers", http.StatusServiceUnavailable)
		return
	}
	defer s.viewers.Add(-1)

	ch := make(chan []byte, 1)
	s.mu.Lock()
	s.subs[ch] = struct{}{}
	s.mu.Unlock()
	defer func() {
		s.mu.Lock()
		delete(s.subs, ch)
		s.mu.Unlock()
	}()

	w.Header().Set("Content-Type", "multipart/x-mixed-replace; boundary=frame")
	// send the headers now, not with the first frame: the viewer knows it is
	// connected even when the camera is slow to produce one
	flusher.Flush()
	rc := http.NewResponseController(w)
	end := time.NewTimer(s.opts.MaxStream)
	defer end.Stop()
	for {
		select {
		case jpeg := <-ch:
			// a viewer that stops reading must not hold this goroutine forever
			rc.SetWriteDeadline(time.Now().Add(10 * time.Second))
			if _, err := fmt.Fprintf(w, "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: %d\r\n\r\n", len(jpeg)); err != nil {
				return
			}
			if _, err := w.Write(jpeg); err != nil {
				return
			}
			if _, err := w.Write([]byte("\r\n")); err != nil {
				return
			}
			flusher.Flush()
		case <-end.C:
			return
		case <-r.Context().Done():
			return
		}
	}
}
