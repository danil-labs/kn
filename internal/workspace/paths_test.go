package workspace

import (
	"path/filepath"
	"testing"
)

func TestSafeJoinBlocksTraversal(t *testing.T) {
	root := t.TempDir()
	if _, err := SafeJoin(root, "../etc/passwd"); err == nil {
		t.Fatal("expected traversal block")
	}
	if _, err := SafeJoin(root, "/etc/passwd"); err == nil {
		t.Fatal("expected abs block")
	}
	got, err := SafeJoin(root, "docs/a.md")
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(root, "docs", "a.md")
	if got != want {
		t.Fatalf("got %s want %s", got, want)
	}
}

func TestIsInternalPath(t *testing.T) {
	if !IsInternalPath(".kn/config.json") {
		t.Fatal("expected .kn internal")
	}
	if !IsInternalPath("foo/.knkeep") {
		t.Fatal("expected keep internal")
	}
	if IsInternalPath("docs/a.md") {
		t.Fatal("doc should not be internal")
	}
}
