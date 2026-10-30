package dailyreport

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestCompose(t *testing.T) {
	sunday := time.Date(2026, 11, 1, 20, 0, 0, 0, time.UTC)
	monday := sunday.AddDate(0, 0, 1)
	at := time.Date(2026, 11, 1, 3, 15, 0, 0, time.UTC)
	tests := []struct {
		name    string
		now     time.Time
		alarms  []time.Time
		paused  bool
		w       *Weather
		want    []string
		notWant []string
	}{
		{"quiet sunny sunday", sunday, nil, false, &Weather{true, 20, 10},
			[]string{"i'm alive, tonight there have been no alarms or movement.", "it's gonna be a great day", "sunny sunday would make a great hike!"}, []string{"at home"}},
		{"sunny monday", monday, nil, false, &Weather{true, 20, 10},
			[]string{"great day"}, []string{"hike"}},
		{"rainy sunday", sunday, nil, false, &Weather{false, 15, 10},
			[]string{"find something to do at home"}, []string{"hike", "great day"}},
		{"one alarm", monday, []time.Time{at}, false, nil,
			[]string{"1 alarm, at 03:15"}, []string{"Today"}},
		{"alarms", monday, []time.Time{at, at.Add(time.Hour)}, false, nil,
			[]string{"2 alarms, the last at 04:15"}, nil},
		{"paused", monday, nil, true, nil, []string{"alarms are paused"}, nil},
		{"hot", monday, nil, false, &Weather{true, 33, 22}, []string{"hot", "sunscreen"}, nil},
		{"cold sunny", monday, nil, false, &Weather{true, 5, -2}, []string{"dress warm"}, nil},
		{"cold rainy", monday, nil, false, &Weather{false, 5, 1}, []string{"at home", "cold"}, nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := Compose(tt.now, tt.alarms, tt.paused, tt.w)
			for _, w := range tt.want {
				if !strings.Contains(got, w) {
					t.Errorf("message misses %q:\n%s", w, got)
				}
			}
			for _, w := range tt.notWant {
				if strings.Contains(got, w) {
					t.Errorf("message has %q:\n%s", w, got)
				}
			}
		})
	}
}

func TestFetchWeather(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("latitude") != "45.5" || r.URL.Query().Get("longitude") != "9.2" {
			t.Errorf("query = %s", r.URL.RawQuery)
		}
		w.Write([]byte(`{"daily":{"weather_code":[1],"temperature_2m_max":[21.4],"temperature_2m_min":[9.8]}}`))
	}))
	defer srv.Close()
	got, err := FetchWeather(context.Background(), srv.Client(), srv.URL, 45.5, 9.2)
	if err != nil {
		t.Fatal(err)
	}
	if *got != (Weather{Sunny: true, MaxC: 21.4, MinC: 9.8}) {
		t.Errorf("got %+v", *got)
	}

	cloudy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`{"daily":{"weather_code":[61],"temperature_2m_max":[12],"temperature_2m_min":[8]}}`))
	}))
	defer cloudy.Close()
	if got, _ := FetchWeather(context.Background(), cloudy.Client(), cloudy.URL, 0, 0); got == nil || got.Sunny {
		t.Errorf("rain reported as %+v", got)
	}

	empty := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write([]byte(`{}`)) }))
	defer empty.Close()
	if _, err := FetchWeather(context.Background(), empty.Client(), empty.URL, 0, 0); err == nil {
		t.Error("empty forecast accepted")
	}

	fail := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { http.Error(w, "no", 500) }))
	defer fail.Close()
	if _, err := FetchWeather(context.Background(), fail.Client(), fail.URL, 0, 0); err == nil {
		t.Error("status 500 accepted")
	}
}

func TestNextRun(t *testing.T) {
	d := func(day, h, m int) time.Time { return time.Date(2026, 10, day, h, m, 0, 0, time.UTC) }
	if got := NextRun(d(30, 13, 0), 20); !got.Equal(d(30, 20, 0)) {
		t.Errorf("before: %v", got)
	}
	if got := NextRun(d(30, 20, 0), 20); !got.Equal(d(31, 20, 0)) {
		t.Errorf("at the hour: %v", got)
	}
	if got := NextRun(d(30, 21, 5), 20); !got.Equal(d(31, 20, 0)) {
		t.Errorf("after: %v", got)
	}
}
