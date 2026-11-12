// Package builddeed sells a Go program with BuildDeed: a license is an NFT
// on NEAR with computer seats, and the license contract keeps the record of
// the official builds.
//
// The program asks one question at start: does this computer have a seat on
// a license that covers this version? Keep the program useful without a
// license, and turn on the extra features when the answer is yes:
//
//	client := builddeed.New(builddeed.Testnet(builddeed.OfficialTestnetContract, "doom", version))
//	lic, err := client.Check(ctx)
//	switch {
//	case err != nil:
//		log.Printf("cannot check the license, running unregistered: %v", err)
//	case lic.Status == builddeed.Registered:
//		log.Printf("registered to %s", lic.AccountID)
//	}
//
// To register the computer, open the register page and wait for the seat.
// The context controls the presence listener and the wait: cancel it when
// the user stops.
//
//	ctx, cancel := context.WithTimeout(context.Background(), builddeed.RegisterWait)
//	defer cancel()
//	if err := client.OpenRegisterPage(ctx); err != nil {
//		log.Printf("cannot open the register page: %v", err)
//	}
//	lic, err = client.WaitForSeat(ctx, builddeed.PollInterval)
//
// There is no offline mode: a result stored on the computer could be copied
// or edited. With no network, run unregistered.
//
// The check is in your public source code. Anyone can build your program
// without it; the official build is the one that your contract records.
//
// The protocol is in spec/README.md of the repository.
package builddeed

import (
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

// OfficialTestnetContract is the contract of the official BuildDeed
// collection on NEAR testnet. All products in it are one NFT collection.
const OfficialTestnetContract = "licenses.builddeed.testnet"

const (
	testnetRPCURL  = "https://rpc.testnet.fastnear.com"
	mainnetRPCURL  = "https://rpc.mainnet.fastnear.com"
	defaultSiteURL = "https://builddeed.dev"
)

// RegisterWait is how long the register page can take: the presence listener
// answers for this time.
const RegisterWait = 10 * time.Minute

// PollInterval is the time between two questions of [Client.WaitForSeat],
// as the spec gives it.
const PollInterval = 5 * time.Second

// Product is the product that this program sells.
type Product struct {
	// Contract is the account of the license contract, for example
	// OfficialTestnetContract.
	Contract string
	// Key is the product key in the contract (add_product), for example "doom".
	Key string
	// Version is the version of this program, as the release records it.
	Version string
	// RPCURL is a NEAR JSON-RPC endpoint of the contract's network. It must
	// use https, except on a loopback address.
	RPCURL string
	// SiteURL is the BuildDeed website, with no slash at the end.
	SiteURL string
}

// Testnet returns a product on NEAR testnet, with the default RPC endpoint
// and website.
func Testnet(contract, key, version string) Product {
	return Product{
		Contract: contract,
		Key:      key,
		Version:  version,
		RPCURL:   testnetRPCURL,
		SiteURL:  defaultSiteURL,
	}
}

// Mainnet returns a product on NEAR mainnet, with the default RPC endpoint
// and website.
func Mainnet(contract, key, version string) Product {
	p := Testnet(contract, key, version)
	p.RPCURL = mainnetRPCURL
	return p
}

// ErrNoMachineID means that the operating system gives no usable machine id,
// so the computer has no id. The program runs unregistered.
var ErrNoMachineID = errors.New("no machine id: run this on the computer that will run the program, not in a container or a cloud sandbox")

// Client is the BuildDeed client of one product on this computer. A Client
// does not change after New, and its methods are safe for concurrent use.
type Client struct {
	product  Product
	deviceID string
	noID     error // why the computer has no id; nil when deviceID is set
}

// New returns a client for p, with the id of this computer. The id is
// computed each time and never stored, so a copied program file does not
// move a seat. When the computer has no id, the methods that need it return
// an error that matches ErrNoMachineID.
func New(p Product) *Client {
	id, err := machineID()
	if err != nil {
		return &Client{product: p, noID: err}
	}
	return &Client{product: p, deviceID: DeviceIDFor(p.Contract, p.Key, id)}
}

// NewWithDevice returns a client with a given computer id (64 lowercase hex
// characters), for tests and for tools that compute the id elsewhere.
func NewWithDevice(p Product, deviceID string) (*Client, error) {
	if !isHex64(deviceID) {
		return nil, fmt.Errorf("a computer id has 64 lowercase hex characters, not %q", deviceID)
	}
	return &Client{product: p, deviceID: deviceID}, nil
}

// isHex64 reports whether s has 64 lowercase hex characters: a computer id
// or a SHA-256.
func isHex64(s string) bool {
	if len(s) != 64 {
		return false
	}
	for i := 0; i < len(s); i++ {
		if !('0' <= s[i] && s[i] <= '9' || 'a' <= s[i] && s[i] <= 'f') {
			return false
		}
	}
	return true
}

// Product returns the product of the client.
func (c *Client) Product() Product { return c.product }

// DeviceID returns the id of this computer: 64 lowercase hex characters.
func (c *Client) DeviceID() (string, error) {
	if c.noID != nil {
		return "", fmt.Errorf("%w: %v", ErrNoMachineID, c.noID)
	}
	return c.deviceID, nil
}

// ShortDeviceID returns the first 8 and the last 4 characters of the
// computer id, as the website shows them: "6844b8b0...af75".
func (c *Client) ShortDeviceID() (string, error) {
	d, err := c.DeviceID()
	if err != nil {
		return "", err
	}
	return d[:8] + "..." + d[len(d)-4:], nil
}

// DeviceIDFor returns the computer id for a contract, a product key and the
// machine id of a computer: the lowercase hex of
// SHA-256("builddeed-device-v1\n" + contract + "\n" + key + "\n" + machine).
func DeviceIDFor(contract, key, machine string) string {
	sum := sha256.Sum256([]byte("builddeed-device-v1\n" + contract + "\n" + key + "\n" + machine))
	return hex.EncodeToString(sum[:])
}

// ConfirmCodeFor returns the 6-digit confirmation code of a computer id: the
// first 4 bytes (big-endian) of SHA-256("builddeed-confirm-v1\n" + deviceID),
// modulo 1 000 000, with leading zeros.
func ConfirmCodeFor(deviceID string) string {
	sum := sha256.Sum256([]byte("builddeed-confirm-v1\n" + deviceID))
	return fmt.Sprintf("%06d", binary.BigEndian.Uint32(sum[:4])%1_000_000)
}

// ConfirmCode returns the code that the program shows while the register
// page is open. Show it with the text "If the page asks for a code, enter
// this one": the page asks for it when the browser blocks the presence check.
func (c *Client) ConfirmCode() (string, error) {
	d, err := c.DeviceID()
	if err != nil {
		return "", err
	}
	return ConfirmCodeFor(d), nil
}

// Status is what the contract says about this computer and this version.
type Status int

const (
	// NoSeat means that the computer has no seat: unregistered.
	NoSeat Status = iota
	// Registered means a seat on a license that covers this version.
	Registered
	// LicenseEnded means a seat, but the license ended before this version:
	// tell the user to renew.
	LicenseEnded
)

// String returns the status in words, for example "license ended".
func (s Status) String() string {
	switch s {
	case NoSeat:
		return "no seat"
	case Registered:
		return "registered"
	case LicenseEnded:
		return "license ended"
	}
	return fmt.Sprintf("Status(%d)", int(s))
}

// License is the answer of the license check. The zero value is NoSeat.
type License struct {
	Status Status
	// AccountID is the owner of the license, for example "alice.testnet".
	AccountID string
	// TokenID is the license (NFT) token id.
	TokenID string
	// UpdatesUntilMs is the end of the update period, in Unix milliseconds:
	// the license covers the builds released at or before this time.
	UpdatesUntilMs int64
}

// Check asks the contract, with one view call, for the seat of this computer
// on a license of the product. Call it at each start. On an error, run
// unregistered and show the error. errors.Is(err, ErrNotExist) is true when
// the contract account does not exist: a wrong contract name.
func (c *Client) Check(ctx context.Context) (License, error) {
	d, err := c.DeviceID()
	if err != nil {
		return License{}, err
	}
	// Pointers: a missing field is an error, never a seat or an ended
	// license.
	var reply *struct {
		TokenID        *string `json:"token_id"`
		OwnerID        *string `json:"owner_id"`
		UpdatesUntilMs *int64  `json:"updates_until_ms"`
		CoversVersion  *bool   `json:"covers_version"`
	}
	args := map[string]string{"product": c.product.Key, "device_id": d, "version": c.product.Version}
	if err := c.view(ctx, "device_license", args, &reply); err != nil {
		return License{}, err
	}
	if reply == nil {
		return License{}, nil
	}
	if reply.TokenID == nil || reply.OwnerID == nil || reply.UpdatesUntilMs == nil || reply.CoversVersion == nil {
		return License{}, &RPCError{Message: "the device_license reply has a missing field"}
	}
	l := License{
		Status:         Registered,
		AccountID:      *reply.OwnerID,
		TokenID:        *reply.TokenID,
		UpdatesUntilMs: *reply.UpdatesUntilMs,
	}
	if !*reply.CoversVersion {
		l.Status = LicenseEnded
	}
	return l, nil
}

// WaitForSeat asks the contract every interval until the computer has a
// seat, and returns that license: Registered, or LicenseEnded when the
// license ended before this version. It tries again after an RPC or network
// error, except ErrNotExist (a wrong contract name). It stops when ctx ends and then returns ctx.Err(): give ctx a
// deadline of RegisterWait, and cancel it when the user stops. Call it after
// OpenRegisterPage, in its own goroutine. An interval of 0 or less is
// PollInterval.
func (c *Client) WaitForSeat(ctx context.Context, interval time.Duration) (License, error) {
	if interval <= 0 {
		interval = PollInterval
	}
	// Errors that a retry cannot repair come back at once.
	if _, err := c.DeviceID(); err != nil {
		return License{}, err
	}
	if err := checkRPCURL(c.product.RPCURL); err != nil {
		return License{}, err
	}
	t := time.NewTicker(interval)
	defer t.Stop()
	var last error
	for {
		if err := ctx.Err(); err != nil {
			if last != nil {
				return License{}, fmt.Errorf("%w (last error: %v)", err, last)
			}
			return License{}, err
		}
		lic, err := c.Check(ctx)
		switch {
		case err == nil && lic.Status != NoSeat:
			return lic, nil
		case errors.Is(err, ErrNotExist):
			return License{}, err
		case err != nil && ctx.Err() == nil:
			// An error that the end of ctx caused is not kept: ctx.Err()
			// reports it.
			last = err
		}
		select {
		case <-ctx.Done():
		case <-t.C:
		}
	}
}

// RegisterURL returns the website page that registers this computer.
// OpenRegisterPage adds the presence check (port and code) to it.
func (c *Client) RegisterURL() (string, error) {
	d, err := c.DeviceID()
	if err != nil {
		return "", err
	}
	p := c.product
	return p.SiteURL + "/register/#product=" + escape(p.Key) +
		"&device=" + escape(d) + "&version=" + escape(p.Version), nil
}

// BuyURL returns the website page that sells a license of the product.
func (c *Client) BuyURL() string {
	return c.product.SiteURL + "/p/" + escape(c.product.Key) + "/"
}

// OpenRegisterPage starts the presence listener and opens the register page
// in the default browser. The listener answers on 127.0.0.1 until ctx ends
// or RegisterWait passes, so give it a context that lives while the page is
// open. The page asks the owner's wallet only when its
// answer names this computer, so a register link from another computer
// cannot give a seat to that computer.
func (c *Client) OpenRegisterPage(ctx context.Context) error {
	return c.openRegisterPage(ctx, openURL)
}

// openRegisterPage is OpenRegisterPage with the function that opens the URL.
func (c *Client) openRegisterPage(ctx context.Context, open func(string) error) error {
	u, err := c.RegisterURL()
	if err != nil {
		return err
	}
	origin, err := originOf(c.product.SiteURL)
	if err != nil {
		return err
	}
	p, err := startPresence(ctx, c.deviceID, c.product.Contract, origin, RegisterWait)
	if err != nil {
		return err
	}
	if err := open(fmt.Sprintf("%s&port=%d&code=%s", u, p.port, p.code)); err != nil {
		p.srv.Close()
		return err
	}
	return nil
}

// OpenBuyPage opens the buy page in the default browser.
func (c *Client) OpenBuyPage() error {
	return openURL(c.BuyURL())
}

// startCommand starts the program that opens a URL, and does not wait for it.
func startCommand(cmd *exec.Cmd) error {
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("cannot open the browser: %w", err)
	}
	// Wait releases the process when it ends. Its result is not used: the
	// program only gives the URL to the browser.
	go func() { _ = cmd.Wait() }()
	return nil
}

// escape percent-encodes all bytes except the unreserved characters of
// RFC 3986, with uppercase hex.
func escape(s string) string {
	const hex = "0123456789ABCDEF"
	var b strings.Builder
	for i := 0; i < len(s); i++ {
		ch := s[i]
		switch {
		case 'A' <= ch && ch <= 'Z', 'a' <= ch && ch <= 'z', '0' <= ch && ch <= '9',
			ch == '-', ch == '_', ch == '.', ch == '~':
			b.WriteByte(ch)
		default:
			b.WriteByte('%')
			b.WriteByte(hex[ch>>4])
			b.WriteByte(hex[ch&15])
		}
	}
	return b.String()
}
