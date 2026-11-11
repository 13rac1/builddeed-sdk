package builddeed

// The presence check: the register page asks the program on this computer
// before it asks the owner's wallet.
//
// A register link carries a computer id. Without a check, someone could send
// a license owner a link with the id of their own computer, and the owner's
// approval would give a seat to a stranger. So while the register page is
// open, the program answers a one-time code on 127.0.0.1:
//
//   - GET /builddeed/<code> answers {"contract": "...", "device_id": "..."}.
//   - Only the origin of the BuildDeed website can read the answer (CORS), so
//     other websites cannot read the computer id.
//   - OPTIONS answers the browser's preflight, with Private Network Access
//     (Access-Control-Allow-Private-Network: true).

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"
)

// maxRequestHead is the largest request line and header that the listener
// reads. net/http reads 4 KiB more than this before it refuses a request.
const maxRequestHead = 8 << 10

// presence is a running presence listener.
type presence struct {
	port int
	code string // the one-time code
	srv  *http.Server
}

// startPresence starts a listener on 127.0.0.1 that answers
// GET /builddeed/<code> for origin, the scheme and host of the website. The
// listener stops when ctx ends, after lifetime, or at p.srv.Close.
func startPresence(ctx context.Context, deviceID, contract, origin string, lifetime time.Duration) (*presence, error) {
	raw := make([]byte, 16)
	if _, err := rand.Read(raw); err != nil {
		return nil, fmt.Errorf("no random source: %w", err)
	}
	code := hex.EncodeToString(raw)
	body, err := json.Marshal(map[string]string{"contract": contract, "device_id": deviceID})
	if err != nil {
		return nil, err
	}
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, fmt.Errorf("cannot start the presence listener: %w", err)
	}
	srv := &http.Server{
		Handler:           presenceHandler("/builddeed/"+code, origin, body),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       5 * time.Second,
		WriteTimeout:      5 * time.Second,
		MaxHeaderBytes:    maxRequestHead,
	}
	srv.SetKeepAlivesEnabled(false)
	ctx, cancel := context.WithTimeout(ctx, lifetime)
	go func() {
		<-ctx.Done()
		srv.Close()
	}()
	go func() {
		// Serve returns when the server closes or the listener fails; cancel
		// then ends the goroutine above.
		_ = srv.Serve(ln)
		cancel()
	}()
	return &presence{port: ln.Addr().(*net.TCPAddr).Port, code: code, srv: srv}, nil
}

// presenceHandler answers the code on path with body, and lets only origin
// read the answers.
func presenceHandler(path, origin string, body []byte) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("Access-Control-Allow-Origin", origin)
		h.Set("Vary", "Origin")
		h.Set("Access-Control-Allow-Methods", "GET")
		h.Set("Access-Control-Allow-Private-Network", "true")
		h.Set("Cache-Control", "no-store")
		switch {
		case r.Method == http.MethodOptions:
			w.WriteHeader(http.StatusNoContent)
		case r.Method == http.MethodGet && subtle.ConstantTimeCompare([]byte(r.URL.Path), []byte(path)) == 1:
			h.Set("Content-Type", "application/json")
			_, _ = w.Write(body)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}
}

// originOf returns the origin of a URL as a browser writes it: the scheme
// and the host in lower case, with no default port. "https://A.b:443/c"
// gives "https://a.b".
func originOf(raw string) (string, error) {
	u, err := url.Parse(raw)
	if err != nil || u.Scheme == "" || u.Hostname() == "" {
		return "", fmt.Errorf("the site URL has no scheme and host: %q", raw)
	}
	scheme, host, port := strings.ToLower(u.Scheme), strings.ToLower(u.Hostname()), u.Port()
	if scheme == "https" && port == "443" || scheme == "http" && port == "80" {
		port = ""
	}
	if port != "" {
		return scheme + "://" + net.JoinHostPort(host, port), nil
	}
	if strings.Contains(host, ":") { // IPv6
		host = "[" + host + "]"
	}
	return scheme + "://" + host, nil
}
