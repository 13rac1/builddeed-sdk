package builddeed

// The shared test data in ../fixtures: every BuildDeed SDK passes it.

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func loadFixture(t *testing.T, name string, v any) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "fixtures", name))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, v); err != nil {
		t.Fatalf("%s: %v", name, err)
	}
}

func TestFixtureDeviceID(t *testing.T) {
	var f struct {
		Cases []struct {
			Contract  string `json:"contract"`
			Product   string `json:"product"`
			MachineID string `json:"machine_id"`
			DeviceID  string `json:"device_id"`
		} `json:"cases"`
	}
	loadFixture(t, "device-id.json", &f)
	if len(f.Cases) == 0 {
		t.Fatal("no cases")
	}
	for _, c := range f.Cases {
		if got := DeviceIDFor(c.Contract, c.Product, c.MachineID); got != c.DeviceID {
			t.Errorf("DeviceIDFor(%q, %q, %q) = %s, want %s", c.Contract, c.Product, c.MachineID, got, c.DeviceID)
		}
	}
}

func TestFixtureConfirmCode(t *testing.T) {
	var f struct {
		Cases []struct {
			DeviceID string `json:"device_id"`
			Code     string `json:"code"`
		} `json:"cases"`
	}
	loadFixture(t, "confirm-code.json", &f)
	if len(f.Cases) == 0 {
		t.Fatal("no cases")
	}
	for _, c := range f.Cases {
		if got := ConfirmCodeFor(c.DeviceID); got != c.Code {
			t.Errorf("ConfirmCodeFor(%s) = %s, want %s", c.DeviceID, got, c.Code)
		}
		got, err := mustClient(Testnet("c", "k", "1"), c.DeviceID).ConfirmCode()
		if err != nil || got != c.Code {
			t.Errorf("ConfirmCode() for %s = %s, %v, want %s", c.DeviceID, got, err, c.Code)
		}
	}
}

func TestFixtureURLs(t *testing.T) {
	var f struct {
		Product struct {
			Contract string `json:"contract"`
			Key      string `json:"key"`
			Version  string `json:"version"`
			SiteURL  string `json:"site_url"`
		} `json:"product"`
		DeviceID      string `json:"device_id"`
		RegisterURL   string `json:"register_url"`
		BuyURL        string `json:"buy_url"`
		ShortDeviceID string `json:"short_device_id"`
	}
	loadFixture(t, "urls.json", &f)
	p := Testnet(f.Product.Contract, f.Product.Key, f.Product.Version)
	p.SiteURL = f.Product.SiteURL
	c := mustClient(p, f.DeviceID)
	if got, err := c.RegisterURL(); err != nil || got != f.RegisterURL {
		t.Errorf("RegisterURL() = %s, %v, want %s", got, err, f.RegisterURL)
	}
	if got := c.BuyURL(); got != f.BuyURL {
		t.Errorf("BuyURL() = %s, want %s", got, f.BuyURL)
	}
	if got, err := c.ShortDeviceID(); err != nil || got != f.ShortDeviceID {
		t.Errorf("ShortDeviceID() = %s, %v, want %s", got, err, f.ShortDeviceID)
	}
}

func TestFixtureDeviceLicense(t *testing.T) {
	var f struct {
		Cases []struct {
			Name           string          `json:"name"`
			Body           json.RawMessage `json:"body"`
			Expect         string          `json:"expect"`
			AccountID      string          `json:"account_id"`
			TokenID        string          `json:"token_id"`
			UpdatesUntilMs int64           `json:"updates_until_ms"`
		} `json:"cases"`
	}
	loadFixture(t, "device-license.json", &f)
	if len(f.Cases) == 0 {
		t.Fatal("no cases")
	}
	for _, c := range f.Cases {
		t.Run(c.Name, func(t *testing.T) {
			rpc := serve(t, reply{200, string(c.Body)})
			p := Testnet(OfficialTestnetContract, "doom", "0.2.0")
			p.RPCURL = rpc.URL
			got, err := mustClient(p, testDevice).Check(context.Background())
			var rpcErr *RPCError
			switch c.Expect {
			case "seat":
				want := License{Status: Registered, AccountID: c.AccountID, TokenID: c.TokenID, UpdatesUntilMs: c.UpdatesUntilMs}
				if err != nil || got != want {
					t.Errorf("Check() = %+v, %v, want %+v", got, err, want)
				}
			case "ended":
				if err != nil || got.Status != LicenseEnded || got.TokenID != c.TokenID || got.UpdatesUntilMs != c.UpdatesUntilMs {
					t.Errorf("Check() = %+v, %v, want LicenseEnded with token %s", got, err, c.TokenID)
				}
			case "none":
				if err != nil || got != (License{}) {
					t.Errorf("Check() = %+v, %v, want NoSeat", got, err)
				}
			case "rpc_error":
				if !errors.As(err, &rpcErr) {
					t.Errorf("Check() = %+v, %v, want an *RPCError", got, err)
				}
			default:
				t.Fatalf("unknown expect %q", c.Expect)
			}
		})
	}
}

// TestFixtureSignedTx encodes the transaction that near-cli signed, from the
// same fields, and checks the bytes and the signature.
func TestFixtureSignedTx(t *testing.T) {
	var f struct {
		SignedTxBase64 string `json:"signed_tx_base64"`
		Signer         string `json:"signer"`
		PublicKey      string `json:"public_key"`
		Receiver       string `json:"receiver"`
		Method         string `json:"method"`
		Args           string `json:"args"`
		Gas            uint64 `json:"gas"`
	}
	loadFixture(t, "signed-tx.json", &f)
	signed, err := base64.StdEncoding.DecodeString(f.SignedTxBase64)
	if err != nil {
		t.Fatal(err)
	}
	if f.Gas != callGas {
		t.Errorf("fixture gas = %d, want %d", f.Gas, callGas)
	}
	public, err := decodeKey(f.PublicKey, ed25519.PublicKeySize)
	if err != nil {
		t.Fatal(err)
	}
	// The nonce and the block hash are read at their places in the bytes.
	atNonce := 4 + len(f.Signer) + 1 + 32
	atHash := atNonce + 8 + 4 + len(f.Receiver)
	tx, err := functionCall{
		signer:    f.Signer,
		publicKey: public,
		nonce:     binary.LittleEndian.Uint64(signed[atNonce:]),
		receiver:  f.Receiver,
		blockHash: signed[atHash : atHash+32],
		method:    f.Method,
		args:      []byte(f.Args),
	}.borsh()
	if err != nil {
		t.Fatal(err)
	}
	if len(signed) != len(tx)+1+ed25519.SignatureSize {
		t.Fatalf("signed transaction has %d bytes, want %d", len(signed), len(tx)+1+ed25519.SignatureSize)
	}
	if string(signed[:len(tx)]) != string(tx) {
		t.Errorf("encoded transaction differs from near-cli:\ngot  %x\nwant %x", tx, signed[:len(tx)])
	}
	if signed[len(tx)] != 0 {
		t.Errorf("signature key type = %d, want 0 (ed25519)", signed[len(tx)])
	}
	sum := sha256.Sum256(tx)
	if !ed25519.Verify(public, sum[:], signed[len(tx)+1:]) {
		t.Error("the near-cli signature does not verify over SHA-256 of the encoded transaction")
	}
	// The SDK writes the same JSON for these arguments.
	args, err := json.Marshal(map[string]string{"product": "checksum", "price": "100000000000000000000000"})
	if err != nil || string(args) != f.Args {
		t.Errorf("args JSON = %s, want %s", args, f.Args)
	}
}

func TestFixtureRPCURLs(t *testing.T) {
	var f struct {
		Accept []string `json:"accept"`
		Refuse []string `json:"refuse"`
	}
	loadFixture(t, "rpc-urls.json", &f)
	if len(f.Accept) == 0 || len(f.Refuse) == 0 {
		t.Fatal("no cases")
	}
	for _, u := range f.Accept {
		if err := checkRPCURL(u); err != nil {
			t.Errorf("checkRPCURL(%q) = %v, want nil", u, err)
		}
	}
	for _, u := range f.Refuse {
		if err := checkRPCURL(u); err == nil {
			t.Errorf("checkRPCURL(%q) = nil, want an error", u)
		}
	}
}

// TestFixtureSendTxResults sends a buy whose send_tx gets each reply of the
// fixture, and checks how the SDK classifies it.
func TestFixtureSendTxResults(t *testing.T) {
	var f struct {
		Cases []struct {
			Name   string          `json:"name"`
			Status int             `json:"status"`
			Body   json.RawMessage `json:"body"`
			Expect string          `json:"expect"`
		} `json:"cases"`
	}
	loadFixture(t, "send-tx-results.json", &f)
	if len(f.Cases) == 0 {
		t.Fatal("no cases")
	}
	for _, c := range f.Cases {
		t.Run(c.Name, func(t *testing.T) {
			body := string(c.Body)
			var text string
			if json.Unmarshal(c.Body, &text) == nil {
				body = text // a string is sent as raw text
			}
			rpc := serve(t, view([]any{}), view(map[string]any{"price": "100"}), accessKey(agentPermission), reply{c.Status, body})
			got, err := testClient(rpc.URL).BuyWithKey(context.Background(), agentKey(t))
			var u *UnconfirmedError
			switch c.Expect {
			case "ok":
				if err != nil || !got.New || got.TokenID == "" {
					t.Errorf("BuyWithKey() = %+v, %v, want a new license", got, err)
				}
			case "refused":
				if err == nil || errors.As(err, &u) {
					t.Errorf("BuyWithKey() error = %v, want a refusal", err)
				}
			case "unconfirmed":
				if !errors.As(err, &u) || u.TxHash == "" {
					t.Errorf("BuyWithKey() error = %v, want *UnconfirmedError with a hash", err)
				}
			default:
				t.Fatalf("unknown expect %q", c.Expect)
			}
		})
	}
}
