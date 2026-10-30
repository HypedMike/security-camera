// Package dailyreport sends a short "I'm alive" message to Telegram every
// day, with the alarms of the night and a weather based suggestion.
package dailyreport

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

// Weather is the forecast of one day.
type Weather struct {
	Sunny      bool
	MaxC, MinC float64
}

// Compose builds the message sent at now. alarms are the times of the alarms
// since the last report, paused says whether alarms are switched off and w is
// today's forecast, nil if it is not known.
func Compose(now time.Time, alarms []time.Time, paused bool, w *Weather) string {
	var b strings.Builder
	b.WriteString("i'm alive, tonight ")
	switch n := len(alarms); {
	case n == 0:
		b.WriteString("there have been no alarms or movement")
	case n == 1:
		fmt.Fprintf(&b, "there has been 1 alarm, at %s", alarms[0].Format("15:04"))
	default:
		fmt.Fprintf(&b, "there have been %d alarms, the last at %s", n, alarms[n-1].Format("15:04"))
	}
	if paused {
		b.WriteString(" (alarms are paused, so some movement may have gone unnoticed)")
	}
	b.WriteString(".")

	if w == nil {
		return b.String()
	}
	b.WriteString(" Today ")
	temp := fmt.Sprintf("%.0f°C to %.0f°C", w.MinC, w.MaxC)
	switch {
	case w.Sunny && w.MaxC >= 30:
		fmt.Fprintf(&b, "it's gonna be a great day, but hot (%s): bring water and sunscreen.", temp)
	case w.Sunny && w.MaxC < 10:
		fmt.Fprintf(&b, "it's gonna be a great day, but cold (%s): dress warm.", temp)
	case w.Sunny:
		fmt.Fprintf(&b, "it's gonna be a great day (%s).", temp)
	case w.MaxC < 10:
		fmt.Fprintf(&b, "you should find something to do at home, it's cold and not sunny (%s).", temp)
	case w.MaxC >= 30:
		fmt.Fprintf(&b, "you should find something to do at home, somewhere cool: it's hot and not sunny (%s).", temp)
	default:
		fmt.Fprintf(&b, "you should find something to do at home, it's not going to be sunny (%s).", temp)
	}
	if w.Sunny && now.Weekday() == time.Sunday {
		b.WriteString(" sunny sunday would make a great hike!")
	}
	return b.String()
}

// sunny reports whether a WMO weather code, as Open-Meteo gives it, is a
// clear sky (0) or a mostly clear one (1).
func sunny(code int) bool { return code == 0 || code == 1 }

// openMeteo is the API the forecast comes from; it needs no key.
const openMeteo = "https://api.open-meteo.com/v1/forecast"

// FetchWeather returns today's forecast for the coordinates. api is the URL of
// the Open-Meteo forecast endpoint, "" for the real one.
func FetchWeather(ctx context.Context, client *http.Client, api string, lat, lon float64) (*Weather, error) {
	if api == "" {
		api = openMeteo
	}
	q := url.Values{
		"latitude":      {fmt.Sprint(lat)},
		"longitude":     {fmt.Sprint(lon)},
		"daily":         {"weather_code,temperature_2m_max,temperature_2m_min"},
		"forecast_days": {"1"},
		"timezone":      {"auto"},
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, api+"?"+q.Encode(), nil)
	if err != nil {
		return nil, err
	}
	resp, err := client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("weather: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		msg, _ := io.ReadAll(io.LimitReader(resp.Body, 256))
		return nil, fmt.Errorf("weather: status %d: %s", resp.StatusCode, msg)
	}
	var data struct {
		Daily struct {
			Code []int     `json:"weather_code"`
			Max  []float64 `json:"temperature_2m_max"`
			Min  []float64 `json:"temperature_2m_min"`
		} `json:"daily"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&data); err != nil {
		return nil, fmt.Errorf("weather: %w", err)
	}
	d := data.Daily
	if len(d.Code) == 0 || len(d.Max) == 0 || len(d.Min) == 0 {
		return nil, errors.New("weather: no forecast for today")
	}
	return &Weather{Sunny: sunny(d.Code[0]), MaxC: d.Max[0], MinC: d.Min[0]}, nil
}

// NextRun returns the next time after now that the clock reads hour:00.
func NextRun(now time.Time, hour int) time.Time {
	next := time.Date(now.Year(), now.Month(), now.Day(), hour, 0, 0, 0, now.Location())
	if !next.After(now) {
		next = time.Date(now.Year(), now.Month(), now.Day()+1, hour, 0, 0, 0, now.Location())
	}
	return next
}

// Run calls send every day at hour:00 local time until ctx is done.
func Run(ctx context.Context, hour int, send func(now time.Time)) {
	for {
		next := NextRun(time.Now(), hour)
		t := time.NewTimer(time.Until(next))
		select {
		case <-ctx.Done():
			t.Stop()
			return
		case <-t.C:
			send(next)
		}
	}
}
