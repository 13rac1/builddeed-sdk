package builddeed

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"time"
)

// maxReply is the largest RPC reply that the client reads.
const maxReply = 4 << 20

// rpcClient sends the JSON-RPC requests. It does not follow redirects: a
// redirect could send the request to a server that is not https.
var rpcClient = &http.Client{
	Timeout: 30 * time.Second,
	CheckRedirect: func(*http.Request, []*http.Request) error {
		return http.ErrUseLastResponse
	},
}

// ErrNotExist means that the RPC node does not know the account or the
// access key (error causes UNKNOWN_ACCOUNT and UNKNOWN_ACCESS_KEY). Match it
// with errors.Is.
var ErrNotExist = errors.New("the account or the access key does not exist")

// RPCError means that the NEAR RPC endpoint answered, but with an error or
// with a reply that is not the expected one. A failed connection is another
// error.
type RPCError struct {
	// Cause is error.cause.name, for example "UNKNOWN_ACCOUNT". It is empty
	// for an error that has no cause name and for an unexpected reply.
	Cause string
	// Message is the text of the error.
	Message string
}

func (e *RPCError) Error() string {
	if e.Cause == "" {
		return "NEAR RPC error: " + e.Message
	}
	return "NEAR RPC error " + e.Cause + ": " + e.Message
}

// Is reports a "does not exist" cause as ErrNotExist.
func (e *RPCError) Is(target error) bool {
	return target == ErrNotExist && (e.Cause == "UNKNOWN_ACCOUNT" || e.Cause == "UNKNOWN_ACCESS_KEY")
}

// checkRPCURL refuses an RPC URL that is not https, so that nobody on the
// network path can answer for the contract. A loopback address can use http,
// for tests and local nodes.
func checkRPCURL(raw string) error {
	u, err := url.Parse(raw)
	if err != nil {
		return fmt.Errorf("bad RPC URL: %w", err)
	}
	if u.Host != "" && (u.Scheme == "https" || u.Scheme == "http" && isLoopback(u.Hostname())) {
		return nil
	}
	return fmt.Errorf("the RPC URL must use https: %s", raw)
}

func isLoopback(host string) bool {
	return host == "127.0.0.1" || host == "::1" || host == "localhost"
}

// post sends one JSON-RPC request to rpcURL and returns its result. An error
// reply or an unexpected reply is an *RPCError; a failed connection is
// another error.
func post(ctx context.Context, rpcURL, method string, params any) (json.RawMessage, error) {
	if err := checkRPCURL(rpcURL); err != nil {
		return nil, err
	}
	body, err := json.Marshal(map[string]any{
		"jsonrpc": "2.0",
		"id":      "builddeed",
		"method":  method,
		"params":  params,
	})
	if err != nil {
		return nil, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, rpcURL, bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := rpcClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("cannot reach the NEAR RPC endpoint: %w", err)
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxReply+1))
	if err != nil {
		return nil, fmt.Errorf("cannot read the NEAR RPC reply: %w", err)
	}
	if len(data) > maxReply {
		return nil, &RPCError{Message: fmt.Sprintf("the reply is larger than %d bytes", maxReply)}
	}
	// nearcore answers some errors with an HTTP error status and a JSON body,
	// so the body decides, not the status.
	var reply struct {
		Result json.RawMessage `json:"result"`
		Error  json.RawMessage `json:"error"`
	}
	if err := json.Unmarshal(data, &reply); err != nil {
		return nil, &RPCError{Message: fmt.Sprintf("the reply (HTTP %d) is not JSON: %v", resp.StatusCode, err)}
	}
	if len(reply.Error) > 0 && string(reply.Error) != "null" {
		// An error with no cause name, or one that is not an object, keeps
		// an empty Cause: it is a failure, not "does not exist".
		var e struct {
			Cause struct {
				Name string `json:"name"`
			} `json:"cause"`
		}
		_ = json.Unmarshal(reply.Error, &e)
		return nil, &RPCError{Cause: e.Cause.Name, Message: string(reply.Error)}
	}
	if len(reply.Result) == 0 || string(reply.Result) == "null" {
		return nil, &RPCError{Message: fmt.Sprintf("the reply (HTTP %d) has no result", resp.StatusCode)}
	}
	// Older nodes put a query error in result.error, as text.
	var legacy struct {
		Error string `json:"error"`
	}
	if json.Unmarshal(reply.Result, &legacy) == nil && legacy.Error != "" {
		return nil, &RPCError{Message: legacy.Error}
	}
	return reply.Result, nil
}

// view calls a view method of the license contract with args, and decodes
// the JSON value that the method returns into out.
func (c *Client) view(ctx context.Context, method string, args map[string]string, out any) error {
	argsJSON, err := json.Marshal(args)
	if err != nil {
		return err
	}
	result, err := post(ctx, c.product.RPCURL, "query", map[string]string{
		"request_type": "call_function",
		"finality":     "final",
		"account_id":   c.product.Contract,
		"method_name":  method,
		"args_base64":  base64.StdEncoding.EncodeToString(argsJSON),
	})
	if err != nil {
		return err
	}
	// The view result is the bytes of a JSON value, as an array of numbers.
	// encoding/json also decodes a base64 string into []byte: refuse it.
	var v struct {
		Result json.RawMessage `json:"result"`
	}
	var b []byte
	if err := json.Unmarshal(result, &v); err != nil || !bytes.HasPrefix(v.Result, []byte("[")) {
		return &RPCError{Message: fmt.Sprintf("the %s result is not an array of bytes", method)}
	}
	if err := json.Unmarshal(v.Result, &b); err != nil {
		return &RPCError{Message: fmt.Sprintf("the %s result is not an array of bytes: %v", method, err)}
	}
	if err := json.Unmarshal(b, out); err != nil {
		return &RPCError{Message: fmt.Sprintf("unexpected %s result: %v", method, err)}
	}
	return nil
}
