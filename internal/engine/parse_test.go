package engine

import "testing"

func TestParseStatusLinePreservesPath(t *testing.T) {
	cases := []struct {
		line string
		path string
		kind string
	}{
		{" M notes.md", "notes.md", "modified"},
		{"M  notes.md", "notes.md", "modified"},
		{"?? new.md", "new.md", "added"},
		{" D gone.md", "gone.md", "deleted"},
		{"A  empty/.knkeep", "empty/", "folder_added"},
	}
	for _, c := range cases {
		got := parseStatusLine(c.line)
		if got.Path != c.path || got.Kind != c.kind {
			t.Fatalf("line %q: got %+v want path=%s kind=%s", c.line, got, c.path, c.kind)
		}
	}
}

func TestCollectPorcelainDoesNotStripLeadingSpace(t *testing.T) {
	out := " M notes.md\n?? other.md\n"
	changes := collectChangesFromPorcelain(out)
	if len(changes) != 2 {
		t.Fatalf("got %+v", changes)
	}
	if changes[0].Path != "notes.md" || changes[1].Path != "other.md" {
		t.Fatalf("got %+v", changes)
	}
}
