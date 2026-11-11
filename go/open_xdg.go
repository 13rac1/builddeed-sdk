//go:build !darwin && !windows

package builddeed

import "os/exec"

// openURL opens url in the default browser. Linux and BSD desktops put
// xdg-open in different folders, so it is found in PATH; it only opens the
// URL.
func openURL(url string) error {
	return startCommand(exec.Command("xdg-open", url))
}
