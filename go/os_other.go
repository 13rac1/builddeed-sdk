//go:build !linux && !darwin && !windows

package builddeed

// machineID: the spec names no machine id for this operating system, so the
// computer has no id.
func machineID() (string, error) {
	return "", errNoUsableID
}
