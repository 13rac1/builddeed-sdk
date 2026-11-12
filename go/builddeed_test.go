package builddeed

import (
	"context"
	"errors"
	"io"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"testing"
	"time"
)

// testDevice is a 64-hex computer id: 6844b8b0, 52 zeros, af75.
const testDevice = "6844b8b0" + "0000000000000000000000000000000000000000000000000000" + "af75"

// testClient returns a client of the product "doom" with testDevice and the
// RPC endpoint rpcURL.
func testClient(rpcURL string) *Client {
	p := Testnet(OfficialTestnetContract, "doom", "0.2.0")
	p.RPCURL = rpcURL
	return mustClient(p, testDevice)
}

func mustClient(p Product, deviceID string) *Client {
	c, err := NewWithDevice(p, deviceID)
	if err != nil {
		panic(err)
	}
	return c
}

func TestProductDefaults(t *testing.T) {
	p := Testnet(OfficialTestnetContract, "doom", "0.2.0")
	if p.RPCURL != "https://rpc.testnet.fastnear.com" || p.SiteURL != "https://builddeed.dev" {
		t.Errorf("Testnet() = %+v", p)
	}
	m := Mainnet("x.near", "doom", "0.2.0")
	if m.RPCURL != "https://rpc.mainnet.fastnear.com" || m.SiteURL != p.SiteURL || m.Key != "doom" {
		t.Errorf("Mainnet() = %+v", m)
	}
}

func TestNewWithDeviceNeedsAComputerID(t *testing.T) {
	for _, id := range []string{"", "é", strings.ToUpper(testDevice), testDevice[:63], testDevice + "0"} {
		if _, err := NewWithDevice(Testnet("c", "k", "1"), id); err == nil {
			t.Errorf("NewWithDevice(%q) error = nil", id)
		}
	}
}

func TestURLsEscapeEachValue(t *testing.T) {
	c := mustClient(Testnet("c", "k&x=1", "1.0 beta"), testDevice)
	got, err := c.RegisterURL()
	if err != nil {
		t.Fatal(err)
	}
	if want := "#product=k%26x%3D1&device=" + testDevice + "&version=1.0%20beta"; !strings.HasSuffix(got, want) {
		t.Errorf("RegisterURL() = %s, want suffix %s", got, want)
	}
	if got := c.BuyURL(); !strings.HasSuffix(got, "/p/k%26x%3D1/") {
		t.Errorf("BuyURL() = %s", got)
	}
}

func TestOpenRegisterPageOpensTheURLOfTheListener(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	c := testClient("http://127.0.0.1:1")
	var opened string
	if err := c.openRegisterPage(ctx, func(u string) error { opened = u; return nil }); err != nil {
		t.Fatal(err)
	}
	base, _ := c.RegisterURL()
	m := regexp.MustCompile(`^&port=(\d+)&code=([0-9a-f]{32})$`).FindStringSubmatch(strings.TrimPrefix(opened, base))
	if !strings.HasPrefix(opened, base) || m == nil {
		t.Fatalf("opened %s, want %s&port=<port>&code=<32 hex>", opened, base)
	}
	resp, err := http.Get("http://127.0.0.1:" + m[1] + "/builddeed/" + m[2])
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	want := `{"contract":"` + OfficialTestnetContract + `","device_id":"` + testDevice + `"}`
	if resp.StatusCode != http.StatusOK || string(body) != want {
		t.Errorf("GET of the opened code = %d %s, want 200 %s", resp.StatusCode, body, want)
	}
}

func TestOpenRegisterPageClosesTheListenerWhenTheBrowserFails(t *testing.T) {
	var port string
	err := testClient("http://127.0.0.1:1").openRegisterPage(context.Background(), func(u string) error {
		port = regexp.MustCompile(`&port=(\d+)`).FindStringSubmatch(u)[1]
		return errors.New("no browser")
	})
	if err == nil {
		t.Fatal("error = nil, want the browser error")
	}
	n, _ := strconv.Atoi(port)
	waitClosed(t, n)
}

func TestNoMachineIDIsAnErrorForDeviceCalls(t *testing.T) {
	c := &Client{product: Testnet("c", "k", "1"), noID: errNoUsableID}
	if _, err := c.Check(context.Background()); !errors.Is(err, ErrNoMachineID) || !strings.Contains(err.Error(), errNoUsableID.Error()) {
		t.Errorf("Check() error = %v, want ErrNoMachineID with its cause", err)
	}
	if _, err := c.RegisterURL(); !errors.Is(err, ErrNoMachineID) {
		t.Errorf("RegisterURL() error = %v, want ErrNoMachineID", err)
	}
	if _, err := c.WaitForSeat(context.Background(), time.Millisecond); !errors.Is(err, ErrNoMachineID) {
		t.Errorf("WaitForSeat() error = %v, want ErrNoMachineID", err)
	}
	if got := c.BuyURL(); !strings.HasSuffix(got, "/p/k/") {
		t.Errorf("BuyURL() = %s", got)
	}
}

func TestCheckAsksForThisComputerAndVersion(t *testing.T) {
	rpc := serve(t, seat("4", "bob.testnet", true))
	if _, err := testClient(rpc.URL).Check(context.Background()); err != nil {
		t.Fatal(err)
	}
	method, params := rpc.request(t, 0)
	if method != "query" || params["method_name"] != "device_license" || params["account_id"] != OfficialTestnetContract {
		t.Errorf("request = %s %v", method, params)
	}
	want := `{"device_id":"` + testDevice + `","product":"doom","version":"0.2.0"}`
	if got := decodeBase64(t, params["args_base64"]); got != want {
		t.Errorf("args = %s, want %s", got, want)
	}
}

func TestPlainHTTPRPCIsRefusedBeforeARequest(t *testing.T) {
	if _, err := testClient("http://rpc.example.com").Check(context.Background()); err == nil || !strings.Contains(err.Error(), "https") {
		t.Errorf("Check() error = %v, want an https error", err)
	}
}

func TestWaitForSeatRetriesThenReturnsTheSeat(t *testing.T) {
	rpc := serve(t, view(nil), rpcError("INTERNAL_ERROR"), reply{503, "busy"}, seat("1", "a.testnet", true))
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	got, err := testClient(rpc.URL).WaitForSeat(ctx, 10*time.Millisecond)
	if err != nil || got.Status != Registered || got.TokenID != "1" {
		t.Errorf("WaitForSeat() = %+v, %v, want the seat on 1", got, err)
	}
}

func TestWaitForSeatReturnsAnEndedLicense(t *testing.T) {
	rpc := serve(t, seat("2", "a.testnet", false))
	got, err := testClient(rpc.URL).WaitForSeat(context.Background(), 10*time.Millisecond)
	if err != nil || got.Status != LicenseEnded || got.TokenID != "2" {
		t.Errorf("WaitForSeat() = %+v, %v, want LicenseEnded on 2", got, err)
	}
}

func TestWaitForSeatStopsWithItsContext(t *testing.T) {
	stopped, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := testClient("http://127.0.0.1:1").WaitForSeat(stopped, time.Millisecond); !errors.Is(err, context.Canceled) {
		t.Errorf("stopped: error = %v, want context.Canceled", err)
	}

	rpc := serve(t, view(nil), view(nil), view(nil), view(nil), view(nil), view(nil), view(nil), view(nil), view(nil), view(nil))
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	if _, err := testClient(rpc.URL).WaitForSeat(ctx, 20*time.Millisecond); !errors.Is(err, context.DeadlineExceeded) {
		t.Errorf("time limit: error = %v, want context.DeadlineExceeded", err)
	}
}

func TestWaitForSeatStopsAtOnceOnAWrongContract(t *testing.T) {
	rpc := serve(t, rpcError("UNKNOWN_ACCOUNT"))
	_, err := testClient(rpc.URL).WaitForSeat(context.Background(), time.Hour)
	if !errors.Is(err, ErrNotExist) {
		t.Errorf("error = %v, want ErrNotExist", err)
	}
}

func TestWaitForSeatReportsTheLastErrorNotTheContext(t *testing.T) {
	rpc := serve(t, rpcError("INTERNAL_ERROR"), rpcError("INTERNAL_ERROR"), rpcError("INTERNAL_ERROR"), rpcError("INTERNAL_ERROR"), rpcError("INTERNAL_ERROR"))
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	_, err := testClient(rpc.URL).WaitForSeat(ctx, 20*time.Millisecond)
	if !errors.Is(err, context.DeadlineExceeded) || !strings.Contains(err.Error(), "INTERNAL_ERROR") {
		t.Errorf("error = %v, want DeadlineExceeded with the last INTERNAL_ERROR", err)
	}
}

func TestWaitForSeatRefusesPlainHTTPAtOnce(t *testing.T) {
	_, err := testClient("http://rpc.example.com").WaitForSeat(context.Background(), time.Hour)
	if err == nil || !strings.Contains(err.Error(), "https") {
		t.Errorf("error = %v, want an https error", err)
	}
}

func TestStatusString(t *testing.T) {
	for s, want := range map[Status]string{NoSeat: "no seat", Registered: "registered", LicenseEnded: "license ended", 7: "Status(7)"} {
		if got := s.String(); got != want {
			t.Errorf("Status(%d).String() = %q, want %q", int(s), got, want)
		}
	}
}
