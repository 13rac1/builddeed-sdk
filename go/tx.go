package builddeed

// Signed function calls, for programs and agents that have no browser.
//
// The transaction is the borsh encoding of a NEAR Transaction with one
// FunctionCall action, signed with ed25519 over its SHA-256, and sent with
// send_tx.

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"slices"
	"strings"
)

// callGas is the gas of one call: enough for a purchase with its transfers.
const callGas uint64 = 100_000_000_000_000

// KeyFile is a NEAR key file as near-cli writes it:
// {"account_id": "...", "public_key": "ed25519:...", "private_key": "ed25519:..."}.
// Its text forms (String, and all fmt verbs) show the account and the public
// key, never the private key.
type KeyFile struct {
	// AccountID is the account of the key. Set it when the file does not
	// name the account.
	AccountID string
	key       ed25519.PrivateKey
}

// LoadKeyFile reads a key file. A key file in
// ~/.near-credentials/<network>/<account>/ has no account_id; LoadKeyFile
// then takes the account from the folder name.
func LoadKeyFile(path string) (KeyFile, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return KeyFile{}, err
	}
	k, err := ParseKeyFile(data)
	if err != nil {
		return KeyFile{}, fmt.Errorf("%s: %w", path, err)
	}
	if k.AccountID == "" {
		if dir := filepath.Base(filepath.Dir(path)); isAccountID(dir) {
			k.AccountID = dir
		}
	}
	return k, nil
}

// ParseKeyFile parses the JSON of a key file, and checks that the private
// key matches the public key.
func ParseKeyFile(data []byte) (KeyFile, error) {
	var raw struct {
		AccountID  string `json:"account_id"`
		PublicKey  string `json:"public_key"`
		PrivateKey string `json:"private_key"`
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		return KeyFile{}, fmt.Errorf("not a NEAR key file: %w", err)
	}
	// near-cli writes the 64-byte form: the seed, then the public key.
	private, err := decodeKey(raw.PrivateKey, ed25519.PrivateKeySize)
	if err != nil {
		return KeyFile{}, fmt.Errorf("private_key: %w", err)
	}
	public, err := decodeKey(raw.PublicKey, ed25519.PublicKeySize)
	if err != nil {
		return KeyFile{}, fmt.Errorf("public_key: %w", err)
	}
	key := ed25519.NewKeyFromSeed(private[:ed25519.SeedSize])
	if !key.Public().(ed25519.PublicKey).Equal(ed25519.PublicKey(public)) {
		return KeyFile{}, errors.New("the private key of the key file does not match its public key")
	}
	return KeyFile{AccountID: raw.AccountID, key: key}, nil
}

// PublicKey returns "ed25519:" and the base58 public key.
func (k KeyFile) PublicKey() string {
	if len(k.key) != ed25519.PrivateKeySize {
		return ""
	}
	return "ed25519:" + base58Encode(k.key[ed25519.SeedSize:])
}

// String returns the account and the public key.
func (k KeyFile) String() string {
	return fmt.Sprintf("KeyFile{AccountID: %q, PublicKey: %q}", k.AccountID, k.PublicKey())
}

// Format writes String for each verb, so that no fmt verb prints the private
// key.
func (k KeyFile) Format(f fmt.State, _ rune) {
	_, _ = f.Write([]byte(k.String()))
}

// isAccountID reports whether a folder name is a NEAR account: a named
// account (with a dot; lowercase letters, digits, "-", "_") or a 64-hex
// implicit account.
func isAccountID(s string) bool {
	if len(s) < 2 || len(s) > 64 || strings.HasPrefix(s, ".") || strings.HasSuffix(s, ".") {
		return false
	}
	for _, c := range s {
		if !('a' <= c && c <= 'z' || '0' <= c && c <= '9' || c == '-' || c == '_' || c == '.') {
			return false
		}
	}
	if strings.Contains(s, ".") {
		return true
	}
	_, err := hex.DecodeString(s)
	return len(s) == 64 && err == nil
}

// decodeKey decodes "ed25519:<base58>" to size bytes.
func decodeKey(text string, size int) ([]byte, error) {
	b58, ok := strings.CutPrefix(text, "ed25519:")
	if !ok {
		return nil, errors.New("only ed25519 keys are supported")
	}
	raw, err := base58Decode(b58)
	if err != nil {
		return nil, err
	}
	if len(raw) != size {
		return nil, fmt.Errorf("the key has %d bytes, not %d", len(raw), size)
	}
	return raw, nil
}

// functionCall is a NEAR Transaction with one FunctionCall action, callGas
// and no deposit.
type functionCall struct {
	signer    string
	publicKey ed25519.PublicKey
	nonce     uint64
	receiver  string
	blockHash []byte // 32 bytes
	method    string
	args      []byte
}

// borsh returns the borsh encoding of the transaction.
func (t functionCall) borsh() ([]byte, error) {
	var out []byte
	put := func(b []byte) error {
		if uint64(len(b)) > math.MaxUint32 {
			return errors.New("a transaction field is too long")
		}
		out = binary.LittleEndian.AppendUint32(out, uint32(len(b)))
		out = append(out, b...)
		return nil
	}
	if len(t.publicKey) != ed25519.PublicKeySize || len(t.blockHash) != 32 {
		return nil, errors.New("a transaction needs a 32-byte public key and block hash")
	}
	if err := put([]byte(t.signer)); err != nil {
		return nil, err
	}
	out = append(out, 0) // KeyType::ED25519
	out = append(out, t.publicKey...)
	out = binary.LittleEndian.AppendUint64(out, t.nonce)
	if err := put([]byte(t.receiver)); err != nil {
		return nil, err
	}
	out = append(out, t.blockHash...)
	out = binary.LittleEndian.AppendUint32(out, 1) // one action
	out = append(out, 2)                           // Action::FunctionCall
	if err := put([]byte(t.method)); err != nil {
		return nil, err
	}
	if err := put(t.args); err != nil {
		return nil, err
	}
	out = binary.LittleEndian.AppendUint64(out, callGas)
	out = append(out, make([]byte, 16)...) // deposit: u128 zero
	return out, nil
}

// sign returns the signed transaction: tx, then the key type and the
// ed25519 signature of SHA-256(tx).
func sign(tx []byte, key ed25519.PrivateKey) []byte {
	sum := sha256.Sum256(tx)
	out := append(slices.Clip(tx), 0) // KeyType::ED25519
	return append(out, ed25519.Sign(key, sum[:])...)
}

// agentMethods are the methods that an agent key may call. A key that allows
// another method, or all methods, is refused: an agent must not hold it.
var agentMethods = []string{"buy_from_budget", "add_device_from_budget"}

// checkAgentKey refuses an access key permission that is not an agent key
// for contract: a full-access key, a key for another contract, or a key that
// allows other methods (an empty method list allows all methods).
func checkAgentKey(permission json.RawMessage, contract string) error {
	var p struct {
		FunctionCall *struct {
			ReceiverID  string   `json:"receiver_id"`
			MethodNames []string `json:"method_names"`
		} `json:"FunctionCall"`
	}
	if string(permission) == `"FullAccess"` {
		return errors.New("this is a full-access key: give the agent a function-call key for buy_from_budget and add_device_from_budget")
	}
	if err := json.Unmarshal(permission, &p); err != nil || p.FunctionCall == nil {
		return fmt.Errorf("unexpected access key permission: %s", permission)
	}
	if p.FunctionCall.ReceiverID != contract {
		return fmt.Errorf("this key is for %q, not for %s", p.FunctionCall.ReceiverID, contract)
	}
	methods := p.FunctionCall.MethodNames
	if len(methods) == 0 || slices.ContainsFunc(methods, func(m string) bool { return !slices.Contains(agentMethods, m) }) {
		return errors.New("this key allows other methods: give the agent a key for buy_from_budget and add_device_from_budget only")
	}
	return nil
}

// UnconfirmedError means that a transaction was sent, but its result is
// unknown: a time-out, a lost connection, or a reply that cannot be read. It
// can have run: read the result (Licenses, Check) before you send it again.
type UnconfirmedError struct {
	// TxHash is the transaction hash, base58.
	TxHash string
	// Err is the cause.
	Err error
}

func (e *UnconfirmedError) Error() string {
	msg := "transaction " + e.TxHash + " has an unknown result; read the result before you send it again"
	if e.Err != nil {
		msg += ": " + e.Err.Error()
	}
	return msg
}

func (e *UnconfirmedError) Unwrap() error { return e.Err }

// refusedCauses are the send_tx error causes (error.cause.name) that mean
// the node refused the transaction, so it did not run.
var refusedCauses = []string{"INVALID_TRANSACTION", "PARSE_ERROR"}

// ErrNoAccount means that the key file names no account. Set
// KeyFile.AccountID.
var ErrNoAccount = errors.New("the key file has no account_id: set KeyFile.AccountID")

// errUnreadable marks a transaction result that cannot be read: the
// transaction ran, but its result is unknown.
var errUnreadable = errors.New("cannot read the transaction result")

// call calls method of the license contract with args, signed with key, and
// waits until the transaction is final. It returns the JSON value that the
// method returns (nil when it returns nothing) and the transaction hash.
// When the transaction was sent but its result is unknown, the error is an
// *UnconfirmedError.
func (c *Client) call(ctx context.Context, key KeyFile, method string, args map[string]string) (json.RawMessage, string, error) {
	if key.AccountID == "" {
		return nil, "", ErrNoAccount
	}
	if len(key.key) != ed25519.PrivateKeySize {
		return nil, "", errors.New("the key file has no key: use LoadKeyFile or ParseKeyFile")
	}
	result, err := post(ctx, c.product.RPCURL, "query", map[string]string{
		"request_type": "view_access_key",
		"finality":     "final",
		"account_id":   key.AccountID,
		"public_key":   key.PublicKey(),
	})
	if err != nil {
		return nil, "", err
	}
	var access struct {
		Nonce      uint64          `json:"nonce"`
		BlockHash  string          `json:"block_hash"`
		Permission json.RawMessage `json:"permission"`
	}
	if err := json.Unmarshal(result, &access); err != nil {
		return nil, "", fmt.Errorf("unexpected access key reply: %w", err)
	}
	if err := checkAgentKey(access.Permission, c.product.Contract); err != nil {
		return nil, "", err
	}
	blockHash, err := base58Decode(access.BlockHash)
	if err != nil || len(blockHash) != 32 {
		return nil, "", errors.New("the access key reply has no block hash")
	}
	argsJSON, err := json.Marshal(args)
	if err != nil {
		return nil, "", err
	}
	tx, err := functionCall{
		signer:    key.AccountID,
		publicKey: key.key.Public().(ed25519.PublicKey),
		nonce:     access.Nonce + 1,
		receiver:  c.product.Contract,
		blockHash: blockHash,
		method:    method,
		args:      argsJSON,
	}.borsh()
	if err != nil {
		return nil, "", err
	}
	sum := sha256.Sum256(tx)
	txHash := base58Encode(sum[:])
	result, err = post(ctx, c.product.RPCURL, "send_tx", map[string]string{
		"signed_tx_base64": base64.StdEncoding.EncodeToString(sign(tx, key.key)),
		"wait_until":       "FINAL",
	})
	if err != nil {
		// Only a node that refused the transaction is sure that it did not
		// run. Any other failure leaves the result unknown.
		var rpcErr *RPCError
		if errors.As(err, &rpcErr) && slices.Contains(refusedCauses, rpcErr.Cause) {
			return nil, txHash, err
		}
		return nil, txHash, &UnconfirmedError{TxHash: txHash, Err: err}
	}
	value, err := outcome(result)
	if errors.Is(err, errUnreadable) {
		return nil, txHash, &UnconfirmedError{TxHash: txHash, Err: err}
	}
	return value, txHash, err
}

// outcome returns the return value of a final transaction, or its failure
// (the contract panicked, so the transaction changed nothing) as an error.
// A result that cannot be read is an error that matches errUnreadable.
func outcome(result json.RawMessage) (json.RawMessage, error) {
	var r struct {
		Status struct {
			SuccessValue *string `json:"SuccessValue"`
			Failure      *struct {
				ActionError struct {
					Kind struct {
						FunctionCallError struct {
							ExecutionError string `json:"ExecutionError"`
						} `json:"FunctionCallError"`
					} `json:"kind"`
				} `json:"ActionError"`
			} `json:"Failure"`
		} `json:"status"`
	}
	if err := json.Unmarshal(result, &r); err != nil {
		return nil, fmt.Errorf("%w: %v", errUnreadable, err)
	}
	switch s := r.Status; {
	case s.SuccessValue != nil:
		if *s.SuccessValue == "" {
			return nil, nil
		}
		v, err := base64.StdEncoding.DecodeString(*s.SuccessValue)
		if err != nil || !json.Valid(v) {
			return nil, fmt.Errorf("%w: the return value is not JSON: %q", errUnreadable, *s.SuccessValue)
		}
		return v, nil
	case s.Failure != nil:
		// The contract's panic message, when there is one.
		msg := s.Failure.ActionError.Kind.FunctionCallError.ExecutionError
		if msg == "" {
			msg = string(result)
		}
		return nil, fmt.Errorf("the contract refused the call: %s", msg)
	}
	return nil, fmt.Errorf("%w: %s", errUnreadable, result)
}
