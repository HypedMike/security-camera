package livegraph

import (
	"bufio"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestEventsSendsBacklogAndLive(t *testing.T) {
	g := New()
	g.Push(0.25)

	mux := httptest.NewServer(g.handler())
	defer mux.Close()

	resp, err := mux.Client().Get(mux.URL + "/events")
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	r := bufio.NewReader(resp.Body)

	read := func() string {
		for {
			line, err := r.ReadString('\n')
			if err != nil {
				t.Fatal(err)
			}
			if strings.HasPrefix(line, "data:") {
				return line
			}
		}
	}
	if l := read(); !strings.HasSuffix(strings.TrimSpace(l), ",0.25]") {
		t.Errorf("backlog: got %q", l)
	}
	go g.Push(0.5)
	if l := read(); !strings.HasSuffix(strings.TrimSpace(l), ",0.5]") {
		t.Errorf("live: got %q", l)
	}
}
