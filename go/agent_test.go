package builddeed

import (
	"bytes"
	"context"
	"encoding/base64"
	"errors"
	"strings"
	"testing"
)

const future = 4_000_000_000_000

func agentKey(t *testing.T) KeyFile {
	t.Helper()
	text, _ := testKeyJSON("owner.testnet", 9)
	k, err := ParseKeyFile([]byte(text))
	if err != nil {
		t.Fatal(err)
	}
	return k
}

func accessKey(permission any) reply {
	return ok(map[string]any{"nonce": 5, "permission": permission, "block_hash": base58Encode(bytes.Repeat([]byte{3}, 32))})
}

var agentPermission = map[string]any{"FunctionCall": map[string]any{
	"allowance": "1", "receiver_id": OfficialTestnetContract,
	"method_names": []string{"buy_from_budget", "add_device_from_budget"},
}}

func license(token string, until int64, devices ...string) map[string]any {
	if devices == nil {
		devices = []string{}
	}
	return map[string]any{"token_id": token, "product": "doom", "owner_id": "owner.testnet",
		"purchased_at_ms": 1, "updates_until_ms": until, "listed_price": nil, "devices": devices}
}

func success(value string) reply {
	return ok(map[string]any{"status": map[string]any{"SuccessValue": base64.StdEncoding.EncodeToString([]byte(value))}})
}

// sentArgs returns the method and the arguments of the transaction in the
// send_tx request i.
func sentArgs(t *testing.T, rpc *fakeRPC, i int) string {
	t.Helper()
	method, params := rpc.request(t, i)
	if method != "send_tx" {
		t.Fatalf("request %d is %s, want send_tx", i, method)
	}
	return decodeBase64(t, params["signed_tx_base64"])
}

func TestBuySendsNothingWhenALicenseIsOwned(t *testing.T) {
	rpc := serve(t, view([]any{license("doom:1", future)}))
	got, err := testClient(rpc.URL).BuyWithKey(context.Background(), agentKey(t))
	if err != nil || got != (Purchase{TokenID: "doom:1"}) {
		t.Errorf("BuyWithKey() = %+v, %v, want doom:1, not new", got, err)
	}
}

func TestBuyPassesThePriceAndReturnsTheNewToken(t *testing.T) {
	rpc := serve(t,
		view([]any{license("doom:1", 1)}),
		view(map[string]any{"price": "100", "max_devices": 3}),
		accessKey(agentPermission),
		success(`{"token_id":"doom:2"}`),
	)
	got, err := testClient(rpc.URL).BuyWithKey(context.Background(), agentKey(t))
	if err != nil || got != (Purchase{TokenID: "doom:2", New: true}) {
		t.Fatalf("BuyWithKey() = %+v, %v, want doom:2, new", got, err)
	}
	tx := sentArgs(t, rpc, 3)
	for _, want := range []string{"owner.testnet", OfficialTestnetContract, "buy_from_budget", `{"price":"100","product":"doom"}`} {
		if !strings.Contains(tx, want) {
			t.Errorf("transaction %q does not contain %q", tx, want)
		}
	}
	// The nonce is the access key nonce plus one.
	if !strings.Contains(tx, "\x06\x00\x00\x00\x00\x00\x00\x00") {
		t.Errorf("transaction %x does not have nonce 6", tx)
	}
}

func TestBuyRefusesAKeyThatIsNotAnAgentKey(t *testing.T) {
	rpc := serve(t, view([]any{}), view(map[string]any{"price": "100"}), accessKey("FullAccess"))
	_, err := testClient(rpc.URL).BuyWithKey(context.Background(), agentKey(t))
	if err == nil || !strings.Contains(err.Error(), "full-access") {
		t.Errorf("BuyWithKey() error = %v, want a full-access error", err)
	}
}

func TestBuyOfAProductThatDoesNotExist(t *testing.T) {
	rpc := serve(t, view([]any{}), view(nil))
	if _, err := testClient(rpc.URL).BuyWithKey(context.Background(), agentKey(t)); err == nil || !strings.Contains(err.Error(), "does not exist") {
		t.Errorf("BuyWithKey() error = %v", err)
	}
}

func TestBuyNeedsTheAccountOfTheKey(t *testing.T) {
	k := agentKey(t)
	k.AccountID = ""
	if _, err := testClient("http://127.0.0.1:1").BuyWithKey(context.Background(), k); !errors.Is(err, ErrNoAccount) {
		t.Errorf("BuyWithKey() error = %v, want ErrNoAccount", err)
	}
	if _, err := testClient("http://127.0.0.1:1").RegisterWithKey(context.Background(), k, ""); !errors.Is(err, ErrNoAccount) {
		t.Errorf("RegisterWithKey() error = %v, want ErrNoAccount", err)
	}
	if _, err := testClient("http://127.0.0.1:1").BuyWithKey(context.Background(), KeyFile{AccountID: "a.testnet"}); err == nil {
		t.Error("BuyWithKey() with an empty KeyFile: error = nil")
	}
}

func TestAnUnknownResultIsUnconfirmedWithItsHash(t *testing.T) {
	tests := []struct {
		name string
		send reply
	}{
		{"time-out error", rpcError("TIMEOUT_ERROR")},
		{"gateway error", reply{502, "<html>Bad Gateway</html>"}},
		{"lost connection", reply{}},
		{"success with no token id", success(`{"other":1}`)},
		{"success that is not JSON", ok(map[string]any{"status": map[string]any{"SuccessValue": "%%%"}})},
		{"status that cannot be read", ok(map[string]any{"status": "Started"})},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			rpc := serve(t, view([]any{}), view(map[string]any{"price": "100"}), accessKey(agentPermission), tt.send)
			_, err := testClient(rpc.URL).BuyWithKey(context.Background(), agentKey(t))
			var u *UnconfirmedError
			if !errors.As(err, &u) {
				t.Fatalf("BuyWithKey() error = %v, want *UnconfirmedError", err)
			}
			tx := []byte(sentArgs(t, rpc, 3))
			signedLen := len(tx) - 1 - 64
			if want := base58Encode(sha256Sum(tx[:signedLen])); u.TxHash != want {
				t.Errorf("TxHash = %s, want %s", u.TxHash, want)
			}
		})
	}
}

func TestAnInvalidTransactionIsRefusedNotUnconfirmed(t *testing.T) {
	rpc := serve(t, view([]any{}), view(map[string]any{"price": "100"}), accessKey(agentPermission), rpcError("INVALID_TRANSACTION"))
	_, err := testClient(rpc.URL).BuyWithKey(context.Background(), agentKey(t))
	var u *UnconfirmedError
	var rpcErr *RPCError
	if errors.As(err, &u) || !errors.As(err, &rpcErr) {
		t.Errorf("BuyWithKey() error = %v, want an *RPCError", err)
	}
}

func TestRegisterReturnsTheSeatItHasAndRefusesAnother(t *testing.T) {
	seated := view([]any{license("doom:1", future, testDevice)})
	rpc := serve(t, seated, seated)
	c := testClient(rpc.URL)
	if got, err := c.RegisterWithKey(context.Background(), agentKey(t), ""); err != nil || got != "doom:1" {
		t.Errorf("RegisterWithKey() = %s, %v, want doom:1", got, err)
	}
	if _, err := c.RegisterWithKey(context.Background(), agentKey(t), "doom:2"); err == nil || !strings.Contains(err.Error(), "doom:1") {
		t.Errorf("RegisterWithKey(doom:2) error = %v, want a seat on doom:1", err)
	}
}

func TestRegisterPicksACurrentLicenseWithAFreeSeat(t *testing.T) {
	full := []string{strings.Repeat("1", 64), strings.Repeat("2", 64), strings.Repeat("3", 64)}
	rpc := serve(t,
		view([]any{license("doom:1", future, full...), license("doom:2", 1), license("doom:3", future)}),
		view(map[string]any{"price": "100", "max_devices": 3}),
		accessKey(agentPermission),
		success(""),
	)
	got, err := testClient(rpc.URL).RegisterWithKey(context.Background(), agentKey(t), "")
	if err != nil || got != "doom:3" {
		t.Fatalf("RegisterWithKey() = %s, %v, want doom:3", got, err)
	}
	want := `{"device_id":"` + testDevice + `","token_id":"doom:3"}`
	if tx := sentArgs(t, rpc, 3); !strings.Contains(tx, want) {
		t.Errorf("transaction %q does not contain %s", tx, want)
	}
}

func TestRegisterWithNoFreeSeat(t *testing.T) {
	rpc := serve(t, view([]any{license("doom:2", 1)}), view(map[string]any{"price": "100", "max_devices": 3}))
	if _, err := testClient(rpc.URL).RegisterWithKey(context.Background(), agentKey(t), ""); err == nil || !strings.Contains(err.Error(), "free seat") {
		t.Errorf("RegisterWithKey() error = %v", err)
	}
}

func TestBudgetAndLicenses(t *testing.T) {
	rpc := serve(t,
		view(map[string]any{"balance": "5", "max_price": "2", "products": []string{"doom"}}),
		view(nil),
		view([]any{license("doom:1", future, testDevice)}),
	)
	c := testClient(rpc.URL)
	ctx := context.Background()
	if b, err := c.Budget(ctx, "owner.testnet"); err != nil || b == nil || b.Balance != "5" || b.MaxPrice != "2" || len(b.Products) != 1 {
		t.Errorf("Budget() = %+v, %v", b, err)
	}
	if b, err := c.Budget(ctx, "owner.testnet"); err != nil || b != nil {
		t.Errorf("Budget() with none = %+v, %v, want nil", b, err)
	}
	ls, err := c.Licenses(ctx, "owner.testnet")
	if err != nil || len(ls) != 1 || ls[0].TokenID != "doom:1" || ls[0].UpdatesUntilMs != future || ls[0].Devices[0] != testDevice {
		t.Errorf("Licenses() = %+v, %v", ls, err)
	}
}
