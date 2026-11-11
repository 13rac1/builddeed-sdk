package builddeed

import (
	"crypto/sha256"
	"testing"
)

func sha256Sum(b []byte) []byte {
	sum := sha256.Sum256(b)
	return sum[:]
}

func TestDeviceIDMatchesAFixedValue(t *testing.T) {
	// printf 'builddeed-device-v1\nc.testnet\np\nm' | sha256sum
	if got := DeviceIDFor("c.testnet", "p", "m"); got != "2c5f892217c817ffa5f7acd469abed07a426cbd850cd240c9d47841e0d2953b0" {
		t.Errorf("DeviceIDFor() = %s", got)
	}
}

func TestUnusableMachineIDs(t *testing.T) {
	for id, want := range map[string]bool{
		"":                 false,
		"uninitialized":    false,
		"0123456789abcde":  false,
		"0123456789abcdef": true,
	} {
		if got := usable(id); got != want {
			t.Errorf("usable(%q) = %v, want %v", id, got, want)
		}
	}
}

func TestParseIoreg(t *testing.T) {
	out := "+-o J314sAP  <class IOPlatformExpertDevice>\n    {\n      \"IOPlatformSerialNumber\" = \"C02XX\"\n      \"IOPlatformUUID\" = \"564D8F2A-1B2C-4D5E-8F90-A1B2C3D4E5F6\"\n    }\n"
	if got := parseIoreg(out); got != "564D8F2A-1B2C-4D5E-8F90-A1B2C3D4E5F6" {
		t.Errorf("parseIoreg() = %q", got)
	}
	if got := parseIoreg("nothing"); got != "" {
		t.Errorf("parseIoreg(nothing) = %q", got)
	}
}

func TestThisComputerHasAnID(t *testing.T) {
	if _, err := machineID(); err != nil {
		t.Skipf("this computer has no machine id: %v", err)
	}
	d, err := New(Testnet(OfficialTestnetContract, "doom", "1")).DeviceID()
	if err != nil || len(d) != 64 {
		t.Errorf("DeviceID() = %s, %v", d, err)
	}
}
