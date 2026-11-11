package builddeed

import (
	"os"
	"strings"
)

// machineID returns the first usable id of /etc/machine-id, then of the
// D-Bus copy: an empty or "uninitialized" /etc/machine-id falls back to it.
func machineID() (string, error) {
	for _, path := range []string{"/etc/machine-id", "/var/lib/dbus/machine-id"} {
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		if id := strings.TrimSpace(string(data)); usable(id) {
			return id, nil
		}
	}
	return "", errNoUsableID
}
