package engine_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/soydanil/kn/internal/cli"
	"github.com/soydanil/kn/internal/engine"
	"github.com/soydanil/kn/internal/workspace"
)

func TestInitStatusDiffSnapshotHistoryRestore(t *testing.T) {
	tmp := t.TempDir()
	home := filepath.Join(tmp, "home")
	if err := os.MkdirAll(home, 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", home)

	root := filepath.Join(tmp, "knowledge")
	if err := os.MkdirAll(root, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "readme.md"), []byte("hello\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(root, "empty-dir"), 0o755); err != nil {
		t.Fatal(err)
	}

	// Preserve a fake user .git and ensure kn does not disturb it
	userGit := filepath.Join(root, ".git")
	if err := os.MkdirAll(userGit, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(userGit, "HEAD"), []byte("ref: refs/heads/main\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	chdir(t, root)

	initRes, err := engine.InitWorkspace(root)
	if err != nil {
		t.Fatalf("init: %v", err)
	}
	if initRes.WorkspaceID == "" || initRes.InitialVersion == "" {
		t.Fatalf("expected workspace id and version: %+v", initRes)
	}
	if _, err := os.Stat(filepath.Join(root, ".kn", "config.json")); err != nil {
		t.Fatal("missing .kn/config.json")
	}
	// No kn-owned .git file should replace user .git dir
	fi, err := os.Stat(userGit)
	if err != nil || !fi.IsDir() {
		t.Fatalf("user .git disturbed: %v", err)
	}
	// Separate git dir under ~/.kn/repos
	repoDir, err := workspace.RepoDir(initRes.WorkspaceID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(repoDir, "HEAD")); err != nil {
		t.Fatalf("missing separate git dir: %v", err)
	}

	// Idempotent init
	init2, err := engine.InitWorkspace(root)
	if err != nil {
		t.Fatal(err)
	}
	if !init2.AlreadyExists || init2.WorkspaceID != initRes.WorkspaceID {
		t.Fatalf("idempotent init failed: %+v", init2)
	}

	// Empty folder should be tracked via .knkeep
	if _, err := os.Stat(filepath.Join(root, "empty-dir", ".knkeep")); err != nil {
		t.Fatalf("expected empty folder marker: %v", err)
	}

	st, err := engine.Status(root)
	if err != nil {
		t.Fatal(err)
	}
	if !st.Clean {
		t.Fatalf("expected clean after init, got changes: %+v", st.LocalChanges)
	}
	if st.RemoteFreshness != "unknown" {
		t.Fatalf("remote freshness: %s", st.RemoteFreshness)
	}
	if st.Capabilities.CloudConnect || st.Capabilities.Pull || st.Capabilities.Push {
		t.Fatalf("cloud caps should be false: %+v", st.Capabilities)
	}

	// Edit a document
	if err := os.WriteFile(filepath.Join(root, "readme.md"), []byte("hello world\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "new.md"), []byte("new\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	st2, err := engine.Status(root)
	if err != nil {
		t.Fatal(err)
	}
	if st2.Clean || len(st2.LocalChanges) == 0 {
		t.Fatalf("expected local changes: %+v", st2.LocalChanges)
	}

	diff, err := engine.Diff(root, false, false)
	if err != nil {
		t.Fatal(err)
	}
	if diff.BaseLabel != "latest_snapshot" {
		t.Fatalf("base label: %s", diff.BaseLabel)
	}
	if len(diff.Changes) == 0 {
		t.Fatal("expected diff changes")
	}

	snap, err := engine.Snapshot(root, "Policy update")
	if err != nil {
		t.Fatal(err)
	}
	if snap.VersionID == "" {
		t.Fatal("missing snapshot version")
	}

	hist, err := engine.History(root, 20)
	if err != nil {
		t.Fatal(err)
	}
	if len(hist.Versions) < 2 {
		t.Fatalf("expected at least init+snapshot versions: %+v", hist.Versions)
	}
	foundManual := false
	for _, v := range hist.Versions {
		if v.Reason == "manual_snapshot" {
			foundManual = true
		}
		if !strings.HasPrefix(v.ID, "v_") {
			t.Fatalf("version id should be abstract: %s", v.ID)
		}
	}
	if !foundManual {
		t.Fatalf("expected manual_snapshot reason: %+v", hist.Versions)
	}

	first := hist.Versions[len(hist.Versions)-1] // oldest = init
	// Modify again then restore
	if err := os.WriteFile(filepath.Join(root, "readme.md"), []byte("changed again\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	rest, err := engine.Restore(root, first.ID)
	if err != nil {
		t.Fatal(err)
	}
	if rest.RestoredVersionID != first.ID {
		t.Fatalf("restored id mismatch: %+v", rest)
	}
	body, err := os.ReadFile(filepath.Join(root, "readme.md"))
	if err != nil {
		t.Fatal(err)
	}
	if string(body) != "hello\n" {
		t.Fatalf("restore content mismatch: %q", body)
	}
	// Sync baseline never rewound (still empty)
	st3, err := workspace.LoadState(root)
	if err != nil {
		t.Fatal(err)
	}
	if st3.SyncBaselineID != "" {
		t.Fatalf("sync baseline should remain empty, got %s", st3.SyncBaselineID)
	}
	if st3.LatestVersionID != rest.NewVersionID {
		t.Fatalf("latest should be new restore version")
	}
}

func TestPathTraversalAndOutboundSymlinkBlocked(t *testing.T) {
	tmp := t.TempDir()
	home := filepath.Join(tmp, "home")
	_ = os.MkdirAll(home, 0o755)
	t.Setenv("HOME", home)
	root := filepath.Join(tmp, "ws")
	_ = os.MkdirAll(root, 0o755)
	_, err := engine.InitWorkspace(root)
	if err != nil {
		t.Fatal(err)
	}
	outside := filepath.Join(tmp, "secret.txt")
	_ = os.WriteFile(outside, []byte("x"), 0o644)
	if err := os.Symlink(outside, filepath.Join(root, "leak")); err != nil {
		t.Fatal(err)
	}
	_, err = engine.Snapshot(root, "should fail")
	if err == nil || !strings.Contains(err.Error(), "symlink") {
		t.Fatalf("expected symlink block, got %v", err)
	}
}

func TestUnsupportedRemoteCommandsExit3(t *testing.T) {
	tmp := t.TempDir()
	home := filepath.Join(tmp, "home")
	_ = os.MkdirAll(home, 0o755)
	t.Setenv("HOME", home)
	root := filepath.Join(tmp, "ws")
	_ = os.MkdirAll(root, 0o755)
	chdir(t, root)
	_, _ = engine.InitWorkspace(root)

	cases := [][]string{
		{"connect", "drive"},
		{"connect", "sharepoint"},
		{"pull"},
		{"push"},
		{"status", "--refresh", "--json"},
	}
	for _, args := range cases {
		code := cli.Run(args, ioDiscard{}, ioDiscard{})
		if code != 3 {
			t.Fatalf("%v: want exit 3, got %d", args, code)
		}
	}
}

func TestJSONEnvelope(t *testing.T) {
	tmp := t.TempDir()
	home := filepath.Join(tmp, "home")
	_ = os.MkdirAll(home, 0o755)
	t.Setenv("HOME", home)
	root := filepath.Join(tmp, "ws")
	_ = os.MkdirAll(root, 0o755)
	chdir(t, root)

	var out strings.Builder
	code := cli.Run([]string{"init", "--json"}, &out, ioDiscard{})
	if code != 0 {
		t.Fatalf("exit %d: %s", code, out.String())
	}
	var env map[string]interface{}
	if err := json.Unmarshal([]byte(out.String()), &env); err != nil {
		t.Fatal(err)
	}
	for _, k := range []string{"schema_version", "operation_id", "status", "data", "conflicts", "errors"} {
		if _, ok := env[k]; !ok {
			t.Fatalf("missing envelope key %s in %v", k, env)
		}
	}
}

type ioDiscard struct{}

func (ioDiscard) Write(p []byte) (int, error) { return len(p), nil }

func chdir(t *testing.T, dir string) {
	t.Helper()
	cwd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Chdir(dir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chdir(cwd) })
}
