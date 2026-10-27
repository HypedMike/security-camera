package livegraph

import (
	_ "embed"
	"fmt"
	"net/http"
	"sync"
	"time"
)

//go:embed index.html
var page []byte

const maxHistory = 300

type point struct {
	t time.Time
	v float64
}

// Graph serves a web page that plots pushed values live.
type Graph struct {
	mu      sync.Mutex
	history []point
	subs    map[chan point]struct{}
}

func New() *Graph {
	return &Graph{subs: map[chan point]struct{}{}}
}

// Push adds a value to the graph. It never blocks: a slow browser drops points.
func (g *Graph) Push(v float64) {
	p := point{time.Now(), v}

	g.mu.Lock()
	defer g.mu.Unlock()
	g.history = append(g.history, p)
	if len(g.history) > maxHistory {
		g.history = g.history[len(g.history)-maxHistory:]
	}
	for ch := range g.subs {
		select {
		case ch <- p:
		default:
		}
	}
}

// ListenAndServe serves the graph page on addr (e.g. ":8080"). It blocks.
func (g *Graph) ListenAndServe(addr string) error {
	return http.ListenAndServe(addr, g.handler())
}

func (g *Graph) handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/" {
			http.NotFound(w, r)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		w.Write(page)
	})
	mux.HandleFunc("/events", g.events)
	return mux
}

// events streams points as server-sent events, starting with the recent history.
func (g *Graph) events(w http.ResponseWriter, r *http.Request) {
	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "streaming unsupported", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")

	ch := make(chan point, 64)
	g.mu.Lock()
	backlog := append([]point(nil), g.history...)
	g.subs[ch] = struct{}{}
	g.mu.Unlock()
	defer func() {
		g.mu.Lock()
		delete(g.subs, ch)
		g.mu.Unlock()
	}()

	send := func(p point) {
		fmt.Fprintf(w, "data: [%d,%g]\n\n", p.t.UnixMilli(), p.v)
	}
	for _, p := range backlog {
		send(p)
	}
	flusher.Flush()

	for {
		select {
		case p := <-ch:
			send(p)
			flusher.Flush()
		case <-r.Context().Done():
			return
		}
	}
}
