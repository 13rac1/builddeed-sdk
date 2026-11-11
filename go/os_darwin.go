package builddeed

import (
	"os/exec"
	"strings"
)

// machineID returns IOPlatformUUID from ioreg.
func machineID() (string, error) {
	out, err := exec.Command("/usr/sbin/ioreg", "-rd1", "-c", "IOPlatformExpertDevice").Output()
	if err != nil {
		return "", err
	}
	if id := strings.TrimSpace(parseIoreg(string(out))); usable(id) {
		return id, nil
	}
	return "", errNoUsableID
}

// openURL opens url in the default browser.
func openURL(url string) error {
	return startCommand(exec.Command("/usr/bin/open", url))
}
