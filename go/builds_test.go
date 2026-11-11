package builddeed

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func record(platform, sha, url string) map[string]any {
	return map[string]any{
		"version": "0.2.0", "commit": strings.Repeat("a", 40), "released_at_ms": 5,
		"artifact": map[string]any{"platform": platform, "file": "dist/app", "sha256": sha, "attestation_url": nil, "url": url},
	}
}

func shaOf(b string) string {
	sum := sha256.Sum256([]byte(b))
	return hex.EncodeToString(sum[:])
}

func TestFindArtifact(t *testing.T) {
	b, c := strings.Repeat("b", 64), strings.Repeat("c", 64)
	rpc := serve(t, view(record("linux-x86_64", b, "")), view(nil), view(record("linux-x86_64", c, "")))
	client := testClient(rpc.URL)
	ctx := context.Background()

	m, err := client.FindArtifact(ctx, strings.ToUpper(b))
	if err != nil || m == nil || m.Version != "0.2.0" || m.ReleasedAtMs != 5 || m.Artifact.File != "dist/app" {
		t.Fatalf("FindArtifact() = %+v, %v", m, err)
	}
	if _, params := rpc.request(t, 0); decodeBase64(t, params["args_base64"]) != `{"product":"doom","sha256":"`+b+`"}` {
		t.Errorf("args = %s", decodeBase64(t, params["args_base64"]))
	}
	if m, err := client.FindArtifact(ctx, c); m != nil || err != nil {
		t.Errorf("FindArtifact(no record) = %+v, %v, want nil, nil", m, err)
	}
	if _, err := client.FindArtifact(ctx, b); err == nil || !strings.Contains(err.Error(), "another file") {
		t.Errorf("FindArtifact(answer for another file) error = %v", err)
	}
	for _, bad := range []string{"xyz", strings.Repeat("g", 64), strings.Repeat("b", 63)} {
		if _, err := client.FindArtifact(ctx, bad); err == nil {
			t.Errorf("FindArtifact(%q) error = nil", bad)
		}
	}
}

func TestLatestBuildRefusesAnotherPlatform(t *testing.T) {
	rpc := serve(t, view(record("windows-x86_64", strings.Repeat("b", 64), "")))
	if _, err := testClient(rpc.URL).LatestBuild(context.Background(), "linux-x86_64"); err == nil {
		t.Error("LatestBuild() error = nil, want a platform error")
	}
}

func TestDownloadLatestRefusesABadRecord(t *testing.T) {
	sha := strings.Repeat("b", 64)
	noFile := record("linux-x86_64", sha, "https://example.com/app")
	noFile["artifact"].(map[string]any)["file"] = "../"
	tests := []struct {
		name  string
		reply reply
		want  string
	}{
		{"no record", view(nil), "no official build"},
		{"bad SHA-256", view(record("linux-x86_64", "xyz", "https://example.com/app")), "SHA-256"},
		{"no URL", view(record("linux-x86_64", sha, "")), "https"},
		{"plain http", view(record("linux-x86_64", sha, "http://example.com/app")), "https"},
		{"no file name", view(noFile), "file name"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			rpc := serve(t, tt.reply)
			_, _, err := testClient(rpc.URL).DownloadLatest(context.Background(), "linux-x86_64", t.TempDir())
			if err == nil || !strings.Contains(err.Error(), tt.want) {
				t.Errorf("error = %v, want %q", err, tt.want)
			}
		})
	}
}

// entries returns the names in dir.
func entries(t *testing.T, dir string) []string {
	t.Helper()
	list, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, e := range list {
		names = append(names, e.Name())
	}
	return names
}

func TestSaveCheckedGivesTheNameOnlyAfterTheCheck(t *testing.T) {
	dir := t.TempDir()
	path, err := saveChecked(strings.NewReader("good"), dir, "app", strings.ToUpper(shaOf("good")), 100)
	if err != nil {
		t.Fatal(err)
	}
	if data, _ := os.ReadFile(path); string(data) != "good" || path != filepath.Join(dir, "app") {
		t.Errorf("saved %s = %q", path, data)
	}
	if got := entries(t, dir); len(got) != 1 {
		t.Errorf("dir has %v, want only app", got)
	}
	if info, err := os.Stat(path); err != nil || runtime.GOOS != "windows" && info.Mode().Perm() != 0o644 {
		t.Errorf("mode of %s = %v, %v, want 0644", path, info.Mode(), err)
	}
}

type brokenReader struct{}

func (brokenReader) Read([]byte) (int, error) { return 0, errors.New("connection reset") }

func TestSaveCheckedFailuresNeverReplaceAFile(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "app"), []byte("old"), 0o600); err != nil {
		t.Fatal(err)
	}
	tests := []struct {
		name  string
		r     io.Reader
		limit int64
	}{
		{"changed file", strings.NewReader("evil"), 100},
		{"broken connection", io.MultiReader(strings.NewReader("go"), brokenReader{}), 100},
		{"larger than the limit", strings.NewReader("good"), 3},
	}
	for _, tt := range tests {
		if _, err := saveChecked(tt.r, dir, "app", shaOf("good"), tt.limit); err == nil {
			t.Errorf("%s: error = nil", tt.name)
		}
	}
	if data, _ := os.ReadFile(filepath.Join(dir, "app")); string(data) != "old" {
		t.Errorf("app = %q, want the old file", data)
	}
	if got := entries(t, dir); len(got) != 1 {
		t.Errorf("dir has %v, want no temporary file", got)
	}
}

// tlsDownload starts an https server with handler, and returns its URL and
// the download client that trusts it.
func tlsDownload(t *testing.T, handler http.Handler) (string, *http.Client) {
	t.Helper()
	srv := httptest.NewTLSServer(handler)
	t.Cleanup(srv.Close)
	hc := newDownloadClient()
	hc.Transport.(*http.Transport).TLSClientConfig = srv.Client().Transport.(*http.Transport).TLSClientConfig
	return srv.URL, hc
}

func TestDownload(t *testing.T) {
	stall := make(chan struct{})
	mux := http.NewServeMux()
	mux.HandleFunc("/good", func(w http.ResponseWriter, _ *http.Request) { io.WriteString(w, "good") })
	mux.HandleFunc("/evil", func(w http.ResponseWriter, _ *http.Request) { io.WriteString(w, "evil") })
	mux.HandleFunc("/to-good", func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, "/good", http.StatusFound) })
	mux.HandleFunc("/to-http", func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, "http://"+r.Host+"/good", http.StatusFound)
	})
	mux.HandleFunc("/loop", func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, "/loop", http.StatusFound) })
	mux.HandleFunc("/stall", func(w http.ResponseWriter, _ *http.Request) {
		io.WriteString(w, "go")
		w.(http.Flusher).Flush()
		<-stall
	})
	base, hc := tlsDownload(t, mux)
	// Registered after the server, so it runs first: the server's Close
	// waits for the stalled handler.
	t.Cleanup(func() { close(stall) })

	tests := []struct {
		path string
		want string // the error text; empty for success
	}{
		{"/good", ""},
		{"/to-good", ""},
		{"/evil", "SHA-256"},
		{"/missing", "HTTP 404"},
		{"/to-http", "away from https"},
		{"/loop", "more than 5 redirects"},
		{"/stall", "no data"},
	}
	for _, tt := range tests {
		t.Run(tt.path, func(t *testing.T) {
			dir := t.TempDir()
			path, err := download(context.Background(), hc, 200*time.Millisecond, base+tt.path, dir, "app", shaOf("good"))
			if tt.want == "" {
				if data, _ := os.ReadFile(path); err != nil || string(data) != "good" {
					t.Errorf("download() = %s, %v; file %q", path, err, data)
				}
				return
			}
			if err == nil || !strings.Contains(err.Error(), tt.want) {
				t.Errorf("error = %v, want %q", err, tt.want)
			}
			if got := entries(t, dir); len(got) != 0 {
				t.Errorf("dir has %v after a failure", got)
			}
		})
	}
}

func TestSHA256File(t *testing.T) {
	path := filepath.Join(t.TempDir(), "f")
	if err := os.WriteFile(path, []byte("one"), 0o600); err != nil {
		t.Fatal(err)
	}
	if got, err := SHA256File(path); err != nil || got != "7692c3ad3540bb803c020b3aee66cd8887123234ea0c6e7143c0add73ff431ed" {
		t.Errorf("SHA256File() = %s, %v", got, err)
	}
}

func TestPlatformUsesRustNames(t *testing.T) {
	tests := []struct{ goos, goarch, want string }{
		{"linux", "amd64", "linux-x86_64"},
		{"windows", "amd64", "windows-x86_64"},
		{"darwin", "arm64", "macos-aarch64"},
		{"linux", "386", "linux-x86"},
		{"linux", "arm", "linux-arm"},
		{"linux", "ppc64le", "linux-powerpc64"},
		{"linux", "ppc64", "linux-powerpc64"},
		{"linux", "loong64", "linux-loongarch64"},
		{"linux", "mips64le", "linux-mips64"},
		{"linux", "mipsle", "linux-mips"},
		{"linux", "riscv64", "linux-riscv64"},
		{"linux", "s390x", "linux-s390x"},
		{"freebsd", "amd64", "freebsd-x86_64"},
		{"wasip1", "wasm", "wasi-wasm32"},
	}
	for _, tt := range tests {
		if got := platform(tt.goos, tt.goarch); got != tt.want {
			t.Errorf("platform(%s, %s) = %s, want %s", tt.goos, tt.goarch, got, tt.want)
		}
	}
}
