package builddeed

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

// Artifact is one file of an official build.
type Artifact struct {
	// Platform is for example "linux-x86_64" (see Platform).
	Platform string `json:"platform"`
	// File is the file name of the release.
	File string `json:"file"`
	// SHA256 is the SHA-256 of the file, lowercase hex.
	SHA256 string `json:"sha256"`
	// AttestationURL is the URL of the provenance attestation, if any.
	AttestationURL string `json:"attestation_url"`
	// URL is the https URL to download the file, if the release recorded one.
	URL string `json:"url"`
}

// ArtifactMatch is an official build and one of its files.
type ArtifactMatch struct {
	// Version is the version of the build.
	Version string `json:"version"`
	// Commit is the git commit of the build.
	Commit string `json:"commit"`
	// ReleasedAtMs is when the build was first recorded, in Unix milliseconds.
	ReleasedAtMs int64 `json:"released_at_ms"`
	// Artifact is the file.
	Artifact Artifact `json:"artifact"`
}

// FindArtifact returns the official build that contains the file with this
// SHA-256 (64 hex characters), or nil when the contract has no record of the
// file. It refuses an answer for another file.
func (c *Client) FindArtifact(ctx context.Context, sha string) (*ArtifactMatch, error) {
	sha = strings.ToLower(sha)
	if !isHex64(sha) {
		return nil, fmt.Errorf("a SHA-256 has 64 hex characters, not %q", sha)
	}
	var m *ArtifactMatch
	if err := c.view(ctx, "find_artifact", map[string]string{"product": c.product.Key, "sha256": sha}, &m); err != nil {
		return nil, err
	}
	if m != nil && strings.ToLower(m.Artifact.SHA256) != sha {
		return nil, errors.New("the contract answered with the record of another file")
	}
	return m, nil
}

// VerifySelf returns the official build record of the running program, or
// nil when the contract has no record of it: for example a copy that someone
// built, or a program that a release ships in an archive whose record is the
// archive.
//
// This is not a security check: a changed program controls its own answer.
// To check a download, hash the file in another program (FindArtifact) or on
// the BuildDeed website.
func (c *Client) VerifySelf(ctx context.Context) (*ArtifactMatch, error) {
	exe, err := os.Executable()
	if err != nil {
		return nil, err
	}
	sha, err := SHA256File(exe)
	if err != nil {
		return nil, err
	}
	return c.FindArtifact(ctx, sha)
}

// LatestBuild returns the last recorded official build that has a file for
// platform, or nil. The release records its main download for each platform
// first, and this is that file.
func (c *Client) LatestBuild(ctx context.Context, platform string) (*ArtifactMatch, error) {
	var m *ArtifactMatch
	if err := c.view(ctx, "latest_build", map[string]string{"product": c.product.Key, "platform": platform}, &m); err != nil {
		return nil, err
	}
	if m != nil && m.Artifact.Platform != platform {
		return nil, errors.New("the contract answered with a file for another platform")
	}
	return m, nil
}

// DownloadLatest downloads the last recorded official file for platform into
// dir, and returns its path and its record. The file goes to a temporary
// name and gets its own name only when its SHA-256 is the recorded one, so a
// failed or changed download never replaces a file. The check trusts the RPC
// endpoint and the contract of the client.
func (c *Client) DownloadLatest(ctx context.Context, platform, dir string) (string, *ArtifactMatch, error) {
	m, err := c.LatestBuild(ctx, platform)
	if err != nil {
		return "", nil, err
	}
	if m == nil {
		return "", nil, fmt.Errorf("no official build for %s", platform)
	}
	if !isHex64(strings.ToLower(m.Artifact.SHA256)) {
		return "", nil, fmt.Errorf("the record has no SHA-256: %q", m.Artifact.SHA256)
	}
	if !strings.HasPrefix(m.Artifact.URL, "https://") {
		return "", nil, fmt.Errorf("the record has no https download URL: %q", m.Artifact.URL)
	}
	name := filepath.Base(m.Artifact.File)
	if name == "." || !filepath.IsLocal(name) {
		return "", nil, fmt.Errorf("the record has no usable file name: %q", m.Artifact.File)
	}
	path, err := download(ctx, newDownloadClient(), downloadIdle, m.Artifact.URL, dir, name, m.Artifact.SHA256)
	if err != nil {
		return "", nil, err
	}
	return path, m, nil
}

const (
	// maxDownload is the largest file that DownloadLatest accepts: 2 GiB.
	maxDownload = 2 << 30
	// downloadIdle is how long a download can receive no data.
	downloadIdle = 60 * time.Second
)

var errStalled = errors.New("the download received no data for too long")

// newDownloadClient returns the HTTP client of DownloadLatest: time limits
// for the connection and the reply, and at most 5 redirects, all https.
func newDownloadClient() *http.Client {
	return &http.Client{
		Transport: &http.Transport{
			Proxy:                 http.ProxyFromEnvironment,
			DialContext:           (&net.Dialer{Timeout: 15 * time.Second}).DialContext,
			TLSHandshakeTimeout:   15 * time.Second,
			ResponseHeaderTimeout: 60 * time.Second,
		},
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) >= 5 {
				return errors.New("the download has more than 5 redirects")
			}
			if req.URL.Scheme != "https" {
				return errors.New("the download was redirected away from https")
			}
			return nil
		},
	}
}

// download gets url with hc and saves it as dir/name when its SHA-256 is sha.
// It stops when no data comes for the time idle.
func download(ctx context.Context, hc *http.Client, idle time.Duration, url, dir, name, sha string) (string, error) {
	ctx, cancel := context.WithCancelCause(ctx)
	defer cancel(nil)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return "", err
	}
	resp, err := hc.Do(req)
	if err != nil {
		return "", fmt.Errorf("cannot download %s: %w", url, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("cannot download %s: HTTP %d", url, resp.StatusCode)
	}
	timer := time.AfterFunc(idle, func() { cancel(errStalled) })
	defer timer.Stop()
	path, err := saveChecked(&idleReader{r: resp.Body, timer: timer, idle: idle}, dir, name, sha, maxDownload)
	if err != nil && errors.Is(context.Cause(ctx), errStalled) {
		return "", errStalled
	}
	return path, err
}

// idleReader restarts timer at each read, so that the timer fires only when
// no data comes for the time idle.
type idleReader struct {
	r     io.Reader
	timer *time.Timer
	idle  time.Duration
}

func (ir *idleReader) Read(p []byte) (int, error) {
	n, err := ir.r.Read(p)
	ir.timer.Reset(ir.idle)
	return n, err
}

// saveChecked writes r to a new temporary file in dir, at most limit bytes,
// and renames it to name only when its SHA-256 is sha. It removes the
// temporary file on each failure.
func saveChecked(r io.Reader, dir, name, sha string, limit int64) (path string, err error) {
	f, err := os.CreateTemp(dir, "."+name+".*.part")
	if err != nil {
		return "", err
	}
	tmp := f.Name()
	defer func() {
		if err != nil {
			f.Close()
			os.Remove(tmp)
		}
	}()
	h := sha256.New()
	n, err := io.Copy(io.MultiWriter(f, h), io.LimitReader(r, limit+1))
	if err != nil {
		return "", fmt.Errorf("download failed: %w", err)
	}
	if n > limit {
		return "", fmt.Errorf("the download is larger than %d bytes", limit)
	}
	if got := hex.EncodeToString(h.Sum(nil)); got != strings.ToLower(sha) {
		return "", fmt.Errorf("the download has SHA-256 %s, not the recorded %s", got, sha)
	}
	// CreateTemp makes the file for its owner only (0600); the downloaded
	// file is for all users to read, as a file that the user saves.
	if err := f.Chmod(0o644); err != nil {
		return "", err
	}
	if err := f.Sync(); err != nil {
		return "", err
	}
	if err := f.Close(); err != nil {
		return "", err
	}
	path = filepath.Join(dir, name)
	if err := os.Rename(tmp, path); err != nil {
		return "", err
	}
	return path, nil
}

// SHA256File returns the SHA-256 of a file, lowercase hex.
func SHA256File(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

// Platform returns the platform name of this computer, as the build records
// use it: "<os>-<arch>" with the Rust names (std::env::consts), for example
// "linux-x86_64", "windows-x86_64" or "macos-aarch64".
func Platform() string {
	return platform(runtime.GOOS, runtime.GOARCH)
}

// rustOS and rustArch map the Go names that differ from the Rust names.
var (
	rustOS = map[string]string{
		"darwin": "macos",
		"wasip1": "wasi",
	}
	rustArch = map[string]string{
		"amd64":    "x86_64",
		"386":      "x86",
		"arm64":    "aarch64",
		"ppc64":    "powerpc64",
		"ppc64le":  "powerpc64",
		"loong64":  "loongarch64",
		"mips64":   "mips64",
		"mips64le": "mips64",
		"mips":     "mips",
		"mipsle":   "mips",
		"wasm":     "wasm32",
	}
)

func platform(goos, goarch string) string {
	if name, ok := rustOS[goos]; ok {
		goos = name
	}
	if name, ok := rustArch[goarch]; ok {
		goarch = name
	}
	return goos + "-" + goarch
}
