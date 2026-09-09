package engine

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/soydanil/kn/internal/workspace"
)

type Change struct {
	Path   string `json:"path"`
	Kind   string `json:"kind"` // added | modified | deleted | folder_added | folder_deleted
	Status string `json:"status,omitempty"`
}

type Version struct {
	ID           string `json:"id"`
	Timestamp    string `json:"timestamp"`
	Reason       string `json:"reason"`
	Message      string `json:"message,omitempty"`
	ChangedDocs  int    `json:"changed_document_count"`
	InternalSHA  string `json:"-"`
}

type InitResult struct {
	WorkspaceID    string `json:"workspace_id"`
	Root           string `json:"root"`
	AlreadyExists  bool   `json:"already_exists"`
	InitialVersion string `json:"initial_version_id,omitempty"`
	Message        string `json:"message"`
}

type StatusResult struct {
	WorkspaceID        string   `json:"workspace_id"`
	LocalChanges       []Change `json:"local_changes"`
	PendingSync        []Change `json:"pending_sync"`
	Conflicts          []string `json:"conflicts"`
	Capabilities       workspace.Caps `json:"capabilities"`
	LastRemoteObserved *string  `json:"last_remote_observed"`
	RemoteFreshness    string   `json:"remote_freshness"`
	LatestVersionID    string   `json:"latest_version_id,omitempty"`
	SyncBaselineID     string   `json:"sync_baseline_id,omitempty"`
	Clean              bool     `json:"clean"`
}

type DiffResult struct {
	BaseVersionID string   `json:"base_version_id,omitempty"`
	BaseLabel     string   `json:"base_label"`
	Changes       []Change `json:"changes"`
	Patch         string   `json:"patch,omitempty"`
}

type HistoryResult struct {
	Versions []Version `json:"versions"`
}

type SnapshotResult struct {
	VersionID string `json:"version_id"`
	Message   string `json:"message"`
	Reason    string `json:"reason"`
	Timestamp string `json:"timestamp"`
	Changed   int    `json:"changed_document_count"`
}

type RestoreResult struct {
	RestoredVersionID string `json:"restored_version_id"`
	NewVersionID      string `json:"new_version_id"`
	PreRestoreSnapID  string `json:"pre_restore_snapshot_id"`
	Message           string `json:"message"`
	SyncBaselineID    string `json:"sync_baseline_id,omitempty"`
}

func openRepoFor(cfg *workspace.Config) (*Repo, error) {
	gitDir, err := workspace.RepoDir(cfg.WorkspaceID)
	if err != nil {
		return nil, err
	}
	return Open(gitDir, cfg.RootPath)
}

// InitWorkspace initializes or returns existing workspace; creates initial snapshot.
func InitWorkspace(root string) (*InitResult, error) {
	abs, err := filepath.Abs(root)
	if err != nil {
		return nil, err
	}
	// Do not disturb existing user .git
	userGit := filepath.Join(abs, ".git")
	hadUserGit := false
	if _, err := os.Lstat(userGit); err == nil {
		hadUserGit = true
	}

	cfg, st, already, err := workspace.EnsureInitialized(abs)
	if err != nil {
		return nil, err
	}

	gitDir, err := workspace.RepoDir(cfg.WorkspaceID)
	if err != nil {
		return nil, err
	}
	repo, err := Init(gitDir, abs)
	if err != nil {
		return nil, err
	}

	// Ensure no .git left in work tree from our init
	if !hadUserGit {
		if fi, err := os.Lstat(userGit); err == nil && !fi.IsDir() {
			_ = os.Remove(userGit)
		}
	}

	res := &InitResult{
		WorkspaceID:   cfg.WorkspaceID,
		Root:          abs,
		AlreadyExists: already,
		Message:       workspace.FormatInitSummary(cfg, already),
	}

	if already && st.LatestVersionID != "" && repo.HasHead() {
		res.InitialVersion = st.LatestVersionID
		return res, nil
	}

	if err := ScanDangerousSymlinks(abs); err != nil {
		return nil, err
	}
	if err := EnsureEmptyFolderMarkers(abs); err != nil {
		return nil, err
	}

	msg := "kn: initial snapshot"
	sha, err := repo.CommitTree(msg)
	if err != nil {
		return nil, err
	}
	vid := VersionIDFromSHA(sha)
	st.LatestVersionID = vid
	if err := workspace.SaveState(abs, st); err != nil {
		return nil, err
	}
	res.InitialVersion = vid
	if already {
		res.Message = workspace.FormatInitSummary(cfg, true)
	} else {
		res.Message = fmt.Sprintf("initialized workspace id=%s version=%s", cfg.WorkspaceID, vid)
	}
	return res, nil
}

func loadWS(start string) (string, *workspace.Config, *workspace.State, *Repo, error) {
	root, err := workspace.FindRoot(start)
	if err != nil {
		return "", nil, nil, nil, err
	}
	cfg, err := workspace.LoadConfig(root)
	if err != nil {
		return "", nil, nil, nil, err
	}
	st, err := workspace.LoadState(root)
	if err != nil {
		st = workspace.DefaultState()
	}
	repo, err := openRepoFor(cfg)
	if err != nil {
		return "", nil, nil, nil, err
	}
	return root, cfg, st, repo, nil
}

func parseStatusLine(line string) Change {
	line = strings.TrimRight(line, "\r\n")
	// git porcelain: two status chars, space, then path — do not TrimSpace the whole line
	if len(line) < 4 {
		return Change{Path: strings.TrimSpace(line), Kind: "modified"}
	}
	xy := line[:2]
	path := strings.TrimSpace(line[3:])
	if strings.Contains(path, " -> ") {
		parts := strings.SplitN(path, " -> ", 2)
		path = parts[len(parts)-1]
	}
	path = filepath.ToSlash(path)
	kind := "modified"
	// Classify from either staged (X) or unstaged (Y) column.
	compact := strings.ReplaceAll(xy, " ", "")
	switch {
	case xy == "??" || strings.ContainsAny(compact, "A"):
		kind = "added"
	case strings.ContainsAny(compact, "D"):
		kind = "deleted"
	case strings.ContainsAny(compact, "MTRC"):
		kind = "modified"
	}
	code := xy
	if workspace.IsKeepMarker(path) {
		folder := workspace.FolderFromKeep(path)
		switch kind {
		case "added":
			return Change{Path: folder, Kind: "folder_added", Status: code}
		case "deleted":
			return Change{Path: folder, Kind: "folder_deleted", Status: code}
		default:
			return Change{Path: folder, Kind: "folder_added", Status: code}
		}
	}
	if workspace.IsInternalPath(path) {
		return Change{}
	}
	return Change{Path: path, Kind: kind, Status: code}
}

func collectChangesFromPorcelain(out string) []Change {
	var changes []Change
	for _, line := range strings.Split(out, "\n") {
		if strings.TrimSpace(line) == "" {
			continue
		}
		c := parseStatusLine(line)
		if c.Path == "" {
			continue
		}
		changes = append(changes, c)
	}
	if changes == nil {
		changes = []Change{}
	}
	return changes
}

func collectChangesFromNameStatus(out string) []Change {
	var changes []Change
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		parts := strings.Fields(line)
		if len(parts) < 2 {
			continue
		}
		code := parts[0]
		path := filepath.ToSlash(parts[len(parts)-1])
		kind := "modified"
		switch code[0] {
		case 'A':
			kind = "added"
		case 'D':
			kind = "deleted"
		case 'M', 'T', 'R', 'C':
			kind = "modified"
		}
		if workspace.IsKeepMarker(path) {
			folder := workspace.FolderFromKeep(path)
			if kind == "deleted" {
				changes = append(changes, Change{Path: folder, Kind: "folder_deleted", Status: code})
			} else {
				changes = append(changes, Change{Path: folder, Kind: "folder_added", Status: code})
			}
			continue
		}
		if workspace.IsInternalPath(path) {
			continue
		}
		changes = append(changes, Change{Path: path, Kind: kind, Status: code})
	}
	if changes == nil {
		changes = []Change{}
	}
	return changes
}

func Status(cwd string) (*StatusResult, error) {
	root, cfg, st, repo, err := loadWS(cwd)
	if err != nil {
		return nil, err
	}
	_ = root
	if err := ScanDangerousSymlinks(cfg.RootPath); err != nil {
		return nil, err
	}
	_ = EnsureEmptyFolderMarkers(cfg.RootPath)

	porcelain, err := repo.StatusPorcelain()
	if err != nil {
		return nil, err
	}
	changes := collectChangesFromPorcelain(porcelain)

	res := &StatusResult{
		WorkspaceID:        cfg.WorkspaceID,
		LocalChanges:       changes,
		PendingSync:        []Change{}, // no cloud in phase 2
		Conflicts:          []string{},
		Capabilities:       st.Capabilities,
		LastRemoteObserved: st.LastRemoteObserved,
		RemoteFreshness:    st.RemoteFreshness,
		LatestVersionID:    st.LatestVersionID,
		SyncBaselineID:     st.SyncBaselineID,
		Clean:              len(changes) == 0,
	}
	return res, nil
}

func Diff(cwd string, useBase bool, includePatch bool) (*DiffResult, error) {
	_, cfg, st, repo, err := loadWS(cwd)
	if err != nil {
		return nil, err
	}
	_ = EnsureEmptyFolderMarkers(cfg.RootPath)

	baseLabel := "latest_snapshot"
	baseRef := "HEAD"
	baseVID := st.LatestVersionID

	if useBase {
		if st.SyncBaselineID == "" {
			return nil, fmt.Errorf("no sync baseline present; connect and sync first (cloud not available in this phase)")
		}
		sha, err := SHAFromVersionID(st.SyncBaselineID)
		if err != nil {
			return nil, err
		}
		full, err := repo.ResolveSHA(sha)
		if err != nil {
			return nil, fmt.Errorf("sync baseline version not found: %s", st.SyncBaselineID)
		}
		baseRef = full
		baseLabel = "sync_baseline"
		baseVID = st.SyncBaselineID
	}

	// Unstaged + untracked vs base: git add -A in index is heavy; use status for working tree vs HEAD,
	// and for --base use diff against baseline SHA including working tree.
	var nameOut string
	if useBase {
		nameOut, err = repo.run("diff", "--name-status", baseRef)
		if err != nil {
			return nil, err
		}
		// Also include untracked
		porc, _ := repo.StatusPorcelain()
		for _, c := range collectChangesFromPorcelain(porc) {
			if c.Kind == "added" || c.Kind == "folder_added" {
				found := false
				for _, existing := range collectChangesFromNameStatus(nameOut) {
					if existing.Path == c.Path {
						found = true
						break
					}
				}
				if !found {
					nameOut += "\nA\t" + strings.TrimSuffix(c.Path, "/")
					if c.Kind == "folder_added" {
						// represent as keep path for parser — simpler append change later
					}
				}
			}
		}
	} else {
		porc, err := repo.StatusPorcelain()
		if err != nil {
			return nil, err
		}
		changes := collectChangesFromPorcelain(porc)
		res := &DiffResult{
			BaseVersionID: baseVID,
			BaseLabel:     baseLabel,
			Changes:       changes,
		}
		if includePatch {
			patch, _ := repo.DiffUnified("")
			res.Patch = patch
		}
		return res, nil
	}

	changes := collectChangesFromNameStatus(nameOut)
	// merge untracked
	porc, _ := repo.StatusPorcelain()
	for _, c := range collectChangesFromPorcelain(porc) {
		if c.Kind != "added" && c.Kind != "folder_added" {
			continue
		}
		found := false
		for _, e := range changes {
			if e.Path == c.Path {
				found = true
				break
			}
		}
		if !found {
			changes = append(changes, c)
		}
	}

	res := &DiffResult{
		BaseVersionID: baseVID,
		BaseLabel:     baseLabel,
		Changes:       changes,
	}
	if includePatch {
		patch, _ := repo.DiffUnified(baseRef)
		res.Patch = patch
	}
	return res, nil
}
