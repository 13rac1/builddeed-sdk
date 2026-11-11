package builddeed

import (
	"errors"
	"fmt"
	"os/exec"
	"syscall"
	"unsafe"
)

// machineID returns MachineGuid from the registry. It reads the registry
// directly, not through reg.exe, so that a changed SystemRoot or PATH cannot
// run another program in its place. KEY_WOW64_64KEY: a 32-bit program must
// read the 64-bit registry view, where the value is.
func machineID() (string, error) {
	path, err := syscall.UTF16PtrFromString(`SOFTWARE\Microsoft\Cryptography`)
	if err != nil {
		return "", err
	}
	var key syscall.Handle
	if err := syscall.RegOpenKeyEx(syscall.HKEY_LOCAL_MACHINE, path, 0, syscall.KEY_READ|syscall.KEY_WOW64_64KEY, &key); err != nil {
		return "", fmt.Errorf("cannot open the Cryptography registry key: %w", err)
	}
	defer syscall.RegCloseKey(key)
	name, err := syscall.UTF16PtrFromString("MachineGuid")
	if err != nil {
		return "", err
	}
	buf := make([]uint16, 128) // a GUID has 36 characters
	size := uint32(len(buf) * 2)
	var typ uint32
	if err := syscall.RegQueryValueEx(key, name, nil, &typ, (*byte)(unsafe.Pointer(&buf[0])), &size); err != nil {
		return "", fmt.Errorf("cannot read MachineGuid: %w", err)
	}
	if typ != syscall.REG_SZ {
		return "", errors.New("MachineGuid is not a string")
	}
	if id := syscall.UTF16ToString(buf[:size/2]); usable(id) {
		return id, nil
	}
	return "", errNoUsableID
}

var getSystemDirectory = syscall.NewLazyDLL("kernel32.dll").NewProc("GetSystemDirectoryW")

// systemDirectory returns the System32 folder from GetSystemDirectoryW, not
// from the environment, which the user can change.
func systemDirectory() (string, error) {
	buf := make([]uint16, syscall.MAX_PATH)
	n, _, err := getSystemDirectory.Call(uintptr(unsafe.Pointer(&buf[0])), uintptr(len(buf)))
	if n == 0 || n >= uintptr(len(buf)) {
		return "", fmt.Errorf("cannot find the System32 folder: %w", err)
	}
	return syscall.UTF16ToString(buf[:n]), nil
}

// openURL opens url in the default browser. Not "cmd /C start": cmd reads
// each "&" in the URL as a command separator.
func openURL(url string) error {
	dir, err := systemDirectory()
	if err != nil {
		return err
	}
	return startCommand(exec.Command(dir+`\rundll32.exe`, "url.dll,FileProtocolHandler", url))
}
