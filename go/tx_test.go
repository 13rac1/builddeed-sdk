package builddeed

import (
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// testKeyJSON returns a key file for a key made from seed. The key is for
// these tests only; it holds nothing.
func testKeyJSON(account string, seed byte) (string, ed25519.PrivateKey) {
	k := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{seed}, ed25519.SeedSize))
	return mustJSON(map[string]string{
		"account_id":  account,
		"public_key":  "ed25519:" + base58Encode(k[32:]),
		"private_key": "ed25519:" + base58Encode(k),
	}), k
}

func TestBase58(t *testing.T) {
	tests := []struct {
		raw []byte
		enc string
	}{
		{nil, ""},
		{[]byte{0}, "1"},
		{[]byte{0, 0, 1}, "112"},
		{[]byte("hello world"), "StV1DL6CwTryKyV"},
		{[]byte{0xff, 0xff}, "LUv"},
	}
	for _, tt := range tests {
		if got := base58Encode(tt.raw); got != tt.enc {
			t.Errorf("base58Encode(%x) = %q, want %q", tt.raw, got, tt.enc)
		}
		if got, err := base58Decode(tt.enc); err != nil || !bytes.Equal(got, tt.raw) {
			t.Errorf("base58Decode(%q) = %x, %v, want %x", tt.enc, got, err, tt.raw)
		}
	}
	for _, bad := range []string{"0", "O", "I", "l", "a b"} {
		if _, err := base58Decode(bad); err == nil {
			t.Errorf("base58Decode(%q) error = nil", bad)
		}
	}
}

func TestParseKeyFile(t *testing.T) {
	text, k := testKeyJSON("agent.testnet", 7)
	f, err := ParseKeyFile([]byte(text))
	if err != nil {
		t.Fatal(err)
	}
	if f.AccountID != "agent.testnet" || !f.key.Equal(k) || f.PublicKey() != "ed25519:"+base58Encode(k[32:]) {
		t.Errorf("ParseKeyFile() = %v", f)
	}
}

func TestParseKeyFileRefusesABadKey(t *testing.T) {
	text, _ := testKeyJSON("agent.testnet", 7)
	var good map[string]string
	if err := json.Unmarshal([]byte(text), &good); err != nil {
		t.Fatal(err)
	}
	change := func(field, value string) string {
		m := map[string]string{}
		for k, v := range good {
			m[k] = v
		}
		m[field] = value
		return mustJSON(m)
	}
	tests := map[string]string{
		"not JSON":            "{",
		"another public key":  change("public_key", "ed25519:"+base58Encode(bytes.Repeat([]byte{1}, 32))),
		"another key type":    change("public_key", "secp256k1:abc"),
		"short private key":   change("private_key", "ed25519:"+base58Encode(bytes.Repeat([]byte{1}, 32))),
		"private key not b58": change("private_key", "ed25519:0OIl"),
	}
	for name, text := range tests {
		if _, err := ParseKeyFile([]byte(text)); err == nil {
			t.Errorf("%s: error = nil", name)
		}
	}
}

func TestKeyFileTextHidesThePrivateKey(t *testing.T) {
	text, k := testKeyJSON("agent.testnet", 7)
	f, err := ParseKeyFile([]byte(text))
	if err != nil {
		t.Fatal(err)
	}
	secret := []string{base58Encode(k), fmt.Sprintf("%x", k[:8]), fmt.Sprint([]byte(k[:8]))[1:12]}
	for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%x", "%d", "%q"} {
		for _, v := range []any{f, &f} {
			out := fmt.Sprintf(verb, v)
			for _, s := range secret {
				if strings.Contains(out, s) {
					t.Errorf("Sprintf(%s, %T) = %s, shows the private key", verb, v, out)
				}
			}
		}
	}
	if out := mustJSON(f); strings.Contains(out, base58Encode(k)) {
		t.Errorf("JSON of a KeyFile = %s, shows the private key", out)
	}
}

func TestLoadKeyFileTakesTheAccountFromTheFolder(t *testing.T) {
	text, _ := testKeyJSON("", 7)
	tests := map[string]string{
		"alice.testnet":         "alice.testnet",
		strings.Repeat("a", 64): strings.Repeat("a", 64),
		"keys":                  "",
		"Alice.testnet":         "",
		"keys.d.":               "",
	}
	for dir, want := range tests {
		path := filepath.Join(t.TempDir(), dir, "key.json")
		if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(text), 0o600); err != nil {
			t.Fatal(err)
		}
		f, err := LoadKeyFile(path)
		if err != nil || f.AccountID != want {
			t.Errorf("LoadKeyFile in %s: account %q, %v, want %q", dir, f.AccountID, err, want)
		}
	}
}

func TestBorshLayout(t *testing.T) {
	tx, err := functionCall{
		signer:    "a",
		publicKey: bytes.Repeat([]byte{1}, 32),
		nonce:     5,
		receiver:  "c",
		blockHash: bytes.Repeat([]byte{2}, 32),
		method:    "m",
		args:      []byte("{}"),
	}.borsh()
	if err != nil {
		t.Fatal(err)
	}
	var want []byte
	want = append(want, 1, 0, 0, 0, 'a', 0)
	want = append(want, bytes.Repeat([]byte{1}, 32)...)
	want = binary.LittleEndian.AppendUint64(want, 5)
	want = append(want, 1, 0, 0, 0, 'c')
	want = append(want, bytes.Repeat([]byte{2}, 32)...)
	want = append(want, 1, 0, 0, 0, 2)
	want = append(want, 1, 0, 0, 0, 'm')
	want = append(want, 2, 0, 0, 0, '{', '}')
	want = binary.LittleEndian.AppendUint64(want, 100_000_000_000_000)
	want = append(want, make([]byte, 16)...)
	if !bytes.Equal(tx, want) {
		t.Errorf("borsh() =\n%x, want\n%x", tx, want)
	}
	if _, err := (functionCall{publicKey: make([]byte, 31), blockHash: make([]byte, 32)}).borsh(); err == nil {
		t.Error("borsh() with a 31-byte key: error = nil")
	}
}

func TestSignatureCoversTheSHA256OfTheTransaction(t *testing.T) {
	_, k := testKeyJSON("a", 7)
	tx := make([]byte, 10, 100) // spare capacity: sign must not write into it
	signed := sign(tx, k)
	if !bytes.Equal(signed[:len(tx)], tx) || signed[len(tx)] != 0 {
		t.Fatalf("signed = %x", signed)
	}
	sum := sha256.Sum256(tx)
	if !ed25519.Verify(k.Public().(ed25519.PublicKey), sum[:], signed[len(tx)+1:]) {
		t.Error("the signature does not verify")
	}
	if got := tx[:cap(tx)][len(tx)]; got != 0 {
		t.Errorf("sign wrote into the spare capacity of tx: %d", got)
	}
}

func TestOnlyAnAgentKeyForTheContractIsUsed(t *testing.T) {
	const c = "licenses.builddeed.testnet"
	fc := func(receiver string, methods ...string) json.RawMessage {
		return json.RawMessage(mustJSON(map[string]any{"FunctionCall": map[string]any{
			"allowance": "1", "receiver_id": receiver, "method_names": methods}}))
	}
	tests := []struct {
		name       string
		permission json.RawMessage
		ok         bool
	}{
		{"full access", json.RawMessage(`"FullAccess"`), false},
		{"no permission", nil, false},
		{"unknown permission", json.RawMessage(`{"Other":{}}`), false},
		{"another contract", fc("other.testnet", "buy_from_budget"), false},
		{"all methods", fc(c), false},
		{"another method", fc(c, "buy_from_budget", "set_prices"), false},
		{"agent key", fc(c, "buy_from_budget", "add_device_from_budget"), true},
		{"buy only", fc(c, "buy_from_budget"), true},
	}
	for _, tt := range tests {
		if err := checkAgentKey(tt.permission, c); (err == nil) != tt.ok {
			t.Errorf("%s: checkAgentKey() = %v, want ok %v", tt.name, err, tt.ok)
		}
	}
}

func TestOutcome(t *testing.T) {
	value := base64.StdEncoding.EncodeToString([]byte(`{"token_id":"doom:1"}`))
	tests := []struct {
		name       string
		result     string
		value      string
		err        string
		unreadable bool
	}{
		{"value", `{"status":{"SuccessValue":"` + value + `"}}`, `{"token_id":"doom:1"}`, "", false},
		{"no value", `{"status":{"SuccessValue":""}}`, "", "", false},
		{"not JSON", `{"status":{"SuccessValue":"` + base64.StdEncoding.EncodeToString([]byte("{")) + `"}}`, "", "not JSON", true},
		{"panic", `{"status":{"Failure":{"ActionError":{"index":0,"kind":{"FunctionCallError":{"ExecutionError":"Smart contract panicked: The agent budget is too low"}}}}}}`, "", "budget is too low", false},
		{"other failure", `{"status":{"Failure":{"InvalidTxError":"InvalidNonce"}}}`, "", "InvalidNonce", false},
		{"no status", `{"status":{}}`, "", "cannot read", true},
		{"status that is not an object", `{"status":"Started"}`, "", "cannot read", true},
	}
	for _, tt := range tests {
		got, err := outcome(json.RawMessage(tt.result))
		if tt.err == "" && (err != nil || string(got) != tt.value) {
			t.Errorf("%s: outcome() = %s, %v, want %s", tt.name, got, err, tt.value)
		}
		if tt.err != "" && (err == nil || !strings.Contains(err.Error(), tt.err)) {
			t.Errorf("%s: outcome() error = %v, want %q", tt.name, err, tt.err)
		}
		if got := errors.Is(err, errUnreadable); got != tt.unreadable {
			t.Errorf("%s: errors.Is(%v, errUnreadable) = %v, want %v", tt.name, err, got, tt.unreadable)
		}
	}
}

func TestUnconfirmedErrorWithNoCause(t *testing.T) {
	if got := (&UnconfirmedError{TxHash: "H"}).Error(); !strings.Contains(got, "H") {
		t.Errorf("Error() = %q", got)
	}
}
