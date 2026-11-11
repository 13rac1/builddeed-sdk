package builddeed

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"time"
)

func TestPresenceAnswersTheCodeForTheSiteOrigin(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	p, err := startPresence(ctx, "d1", "c.testnet", "https://13rac1.github.io", time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	port, code := p.port, p.code
	if len(code) != 32 {
		t.Errorf("code %q has %d characters, want 32", code, len(code))
	}
	base := fmt.Sprintf("http://127.0.0.1:%d", port)
	tests := []struct {
		method, path string
		status       int
		body         string
	}{
		{http.MethodGet, "/builddeed/" + code, 200, `{"contract":"c.testnet","device_id":"d1"}`},
		{http.MethodGet, "/builddeed/" + strings.Repeat("0", 32), 404, ""},
		{http.MethodGet, "/", 404, ""},
		{http.MethodPost, "/builddeed/" + code, 404, ""},
		{http.MethodOptions, "/builddeed/" + code, 204, ""},
	}
	for _, tt := range tests {
		req, err := http.NewRequest(tt.method, base+tt.path, nil)
		if err != nil {
			t.Fatal(err)
		}
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatalf("%s %s: %v", tt.method, tt.path, err)
		}
		body, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		if resp.StatusCode != tt.status || string(body) != tt.body {
			t.Errorf("%s %s = %d %q, want %d %q", tt.method, tt.path, resp.StatusCode, body, tt.status, tt.body)
		}
		for name, want := range map[string]string{
			"Access-Control-Allow-Origin":          "https://13rac1.github.io",
			"Access-Control-Allow-Private-Network": "true",
			"Cache-Control":                        "no-store",
		} {
			if got := resp.Header.Get(name); got != want {
				t.Errorf("%s %s: %s = %q, want %q", tt.method, tt.path, name, got, want)
			}
		}
	}
}

func TestPresenceRefusesALongRequestLine(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	p, err := startPresence(ctx, "d", "c", "https://x", time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	resp, err := http.Get(fmt.Sprintf("http://127.0.0.1:%d/%s", p.port, strings.Repeat("a", 64<<10)))
	if err != nil {
		return // the listener closed the connection: also a refusal
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusRequestHeaderFieldsTooLarge {
		t.Errorf("status = %d, want 431", resp.StatusCode)
	}
}

func TestPresenceStopsWithItsContextItsLifetimeAndClose(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	stopped, err := startPresence(ctx, "d", "c", "https://x", time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	cancel()
	expired, err := startPresence(context.Background(), "d", "c", "https://x", time.Millisecond)
	if err != nil {
		t.Fatal(err)
	}
	closed, err := startPresence(context.Background(), "d", "c", "https://x", time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	closed.srv.Close()
	for _, p := range []*presence{stopped, expired, closed} {
		waitClosed(t, p.port)
	}
}

// waitClosed waits until nothing listens on port.
func waitClosed(t *testing.T, port int) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		conn, err := net.Dial("tcp", fmt.Sprintf("127.0.0.1:%d", port))
		if err != nil {
			return
		}
		conn.Close()
		time.Sleep(10 * time.Millisecond)
	}
	t.Errorf("port %d still accepts connections", port)
}

func TestOriginIsSchemeAndHost(t *testing.T) {
	for in, want := range map[string]string{
		"https://13rac1.github.io/builddeed":     "https://13rac1.github.io",
		"HTTPS://13Rac1.GitHub.io:443/builddeed": "https://13rac1.github.io",
		"http://127.0.0.1:8099":                  "http://127.0.0.1:8099",
		"http://example.com:80/":                 "http://example.com",
		"https://[::1]:8443/x":                   "https://[::1]:8443",
		"https://[::1]/x":                        "https://[::1]",
	} {
		if got, err := originOf(in); err != nil || got != want {
			t.Errorf("originOf(%s) = %s, %v, want %s", in, got, err, want)
		}
	}
	if _, err := originOf("13rac1.github.io"); err == nil {
		t.Error("originOf with no scheme: error = nil")
	}
}
