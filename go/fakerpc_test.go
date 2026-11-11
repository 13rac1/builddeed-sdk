package builddeed

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
)

// reply is one answer of the fake RPC endpoint. The zero reply closes the
// connection with no answer.
type reply struct {
	status int
	body   string
}

// fakeRPC is a loopback NEAR RPC endpoint that answers one request with
// each reply, in order, and keeps the request bodies.
type fakeRPC struct {
	URL string

	mu       sync.Mutex
	replies  []reply
	requests []json.RawMessage
}

func serve(t *testing.T, replies ...reply) *fakeRPC {
	t.Helper()
	f := &fakeRPC{replies: replies}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		f.mu.Lock()
		defer f.mu.Unlock()
		f.requests = append(f.requests, body)
		if len(f.replies) == 0 {
			t.Errorf("unexpected RPC request %d: %s", len(f.requests), body)
			w.WriteHeader(http.StatusInternalServerError)
			return
		}
		next := f.replies[0]
		f.replies = f.replies[1:]
		if next.status == 0 {
			if conn, _, err := w.(http.Hijacker).Hijack(); err == nil {
				conn.Close()
			}
			return
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(next.status)
		_, _ = io.WriteString(w, next.body)
	}))
	t.Cleanup(srv.Close)
	f.URL = srv.URL
	return f
}

// request returns the params of request i, decoded.
func (f *fakeRPC) request(t *testing.T, i int) (method string, params map[string]any) {
	t.Helper()
	f.mu.Lock()
	defer f.mu.Unlock()
	if i >= len(f.requests) {
		t.Fatalf("got %d requests, want more than %d", len(f.requests), i)
	}
	var r struct {
		Method string         `json:"method"`
		Params map[string]any `json:"params"`
	}
	if err := json.Unmarshal(f.requests[i], &r); err != nil {
		t.Fatalf("request %d is not JSON: %v", i, err)
	}
	return r.Method, r.Params
}

func mustJSON(v any) string {
	b, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	return string(b)
}

// ok is a JSON-RPC reply with result.
func ok(result any) reply {
	return reply{200, mustJSON(map[string]any{"jsonrpc": "2.0", "id": "x", "result": result})}
}

// rpcError is a JSON-RPC error reply whose cause has this name.
func rpcError(cause string) reply {
	return reply{200, mustJSON(map[string]any{"jsonrpc": "2.0", "id": "x", "error": map[string]any{
		"name": "HANDLER_ERROR", "cause": map[string]any{"name": cause, "info": map[string]any{}},
		"code": -32000, "message": "Server error",
	}})}
}

// view is the reply of a view call that returns v.
func view(v any) reply {
	b := []byte(mustJSON(v))
	nums := make([]int, len(b))
	for i, c := range b {
		nums[i] = int(c)
	}
	return ok(map[string]any{"result": nums, "logs": []string{}})
}

// seat is the reply of device_license for a computer with a seat.
func seat(token, owner string, covers bool) reply {
	return view(map[string]any{"token_id": token, "owner_id": owner, "updates_until_ms": int64(1825882492821), "covers_version": covers})
}
