package builddeed

import (
	"context"
	"encoding/base64"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"testing"
)

func decodeBase64(t *testing.T, v any) string {
	t.Helper()
	s, _ := v.(string)
	b, err := base64.StdEncoding.DecodeString(s)
	if err != nil {
		t.Fatalf("not base64: %v", v)
	}
	return string(b)
}

func TestRPCErrorsAreClassifiedByCause(t *testing.T) {
	tests := []struct {
		name     string
		reply    reply
		notExist bool
	}{
		{"unknown account", rpcError("UNKNOWN_ACCOUNT"), true},
		{"unknown access key", rpcError("UNKNOWN_ACCESS_KEY"), true},
		{"internal error", rpcError("INTERNAL_ERROR"), false},
		{"HTTP error with a JSON body", reply{400, `{"error":{"cause":{"name":"UNKNOWN_ACCOUNT"}}}`}, true},
		{"error with no cause", reply{200, `{"error":"bad request"}`}, false},
		{"legacy text error", ok(map[string]string{"error": "wasm execution failed"}), false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			rpc := serve(t, tt.reply)
			_, err := testClient(rpc.URL).Check(context.Background())
			var rpcErr *RPCError
			if !errors.As(err, &rpcErr) {
				t.Fatalf("error = %v, want an *RPCError", err)
			}
			if got := errors.Is(err, ErrNotExist); got != tt.notExist {
				t.Errorf("errors.Is(%v, ErrNotExist) = %v, want %v", err, got, tt.notExist)
			}
		})
	}
}

func TestUnexpectedRepliesAreRPCErrors(t *testing.T) {
	tests := []struct {
		name  string
		reply reply
	}{
		{"HTTP error with no JSON body", reply{503, "Service Unavailable"}},
		{"no result", reply{200, `{"jsonrpc":"2.0","id":"x"}`}},
		{"view result that is not bytes", ok(map[string]any{"result": "x"})},
		{"view result as base64", ok(map[string]any{"result": base64.StdEncoding.EncodeToString([]byte("null"))})},
		{"view result with a number over 255", ok(map[string]any{"result": []int{256}})},
		{"view result that is not JSON", ok(map[string]any{"result": []int{'{'}})},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			rpc := serve(t, tt.reply)
			_, err := testClient(rpc.URL).Check(context.Background())
			var rpcErr *RPCError
			if !errors.As(err, &rpcErr) {
				t.Errorf("error = %v, want an *RPCError", err)
			}
		})
	}
}

func TestNoNetworkIsNotAnRPCError(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := ln.Addr().String()
	ln.Close()
	_, err = testClient("http://" + addr).Check(context.Background())
	var rpcErr *RPCError
	if err == nil || errors.As(err, &rpcErr) {
		t.Errorf("Check() with no server: error = %v, want a connection error", err)
	}
}

func TestRPCDoesNotFollowRedirects(t *testing.T) {
	srv := httptest.NewServer(http.RedirectHandler("http://rpc.example.com/", http.StatusTemporaryRedirect))
	defer srv.Close()
	if _, err := testClient(srv.URL).Check(context.Background()); err == nil {
		t.Error("Check() after a redirect: error = nil")
	}
}
