package builddeed

// The computer id is the SHA-256 of the operating system's machine id, the
// license contract, the product key and a fixed label (DeviceIDFor). The
// program computes it at each start and never stores it, so a copied program
// file does not move a seat to another computer.
//
// Limit: a user with root or administrator rights can set the machine id, so
// a machine id copied from a computer with a seat registers another
// computer. Computers cloned from one disk image share an id. The helper
// programs are called by absolute path, so that a program of the same name
// earlier in PATH, or next to the program, cannot answer in their place;
// Windows reads the registry with no helper program.

import (
	"errors"
	"strings"
)

var errNoUsableID = errors.New("no usable machine id")

// usable reports whether a machine id can make a computer id: 16 characters
// or more, and not the placeholder of an image that systemd did not set up.
func usable(id string) bool {
	return len(id) >= 16 && id != "uninitialized"
}

// parseIoreg returns the value of `"IOPlatformUUID" = "564D8F2A-..."` in the
// output of ioreg.
func parseIoreg(out string) string {
	for _, line := range strings.Split(out, "\n") {
		if !strings.Contains(line, `"IOPlatformUUID"`) {
			continue
		}
		_, value, ok := strings.Cut(line, "=")
		if !ok {
			return ""
		}
		return strings.Trim(strings.TrimSpace(value), `"`)
	}
	return ""
}
