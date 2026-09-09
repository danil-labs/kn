package engine

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/soydanil/kn/internal/workspace"
)

func Snapshot(cwd, message string) (*SnapshotResult, error) {
	_, cfg, st, repo, err := loadWS(cwd)
	if err != nil {
		return nil, err
	}
	if err := ScanDangerousSymlinks(cfg.RootPath); err != nil {
		return nil, err
	}
	if err := EnsureEmptyFolderMarkers(cfg.RootPath); err != nil {
		return nil, err
	}
	reason := "manual_snapshot"
	if message == "" {
		message = "kn: snapshot"
	} else {
		message = "kn: " + message
	}
	// Count changes before commit
	porc, _ := repo.StatusPorcelain()
	preChanges := collectChangesFromPorcelain(porc)

	sha, err := repo.CommitTree(message)
	if err != nil {
		return nil, err
	}
	vid := VersionIDFromSHA(sha)
	st.LatestVersionID = vid
	if err := workspace.SaveState(cfg.RootPath, st); err != nil {
		return nil, err
	}
	return &SnapshotResult{
		VersionID: vid,
		Message:   message,
		Reason:    reason,
		Timestamp: time.Now().UTC().Format(time.RFC3339),
		Changed:   len(preChanges),
	}, nil
}

func History(cwd string, limit int) (*HistoryResult, error) {
	_, _, _, repo, err := loadWS(cwd)
	if err != nil {
		return nil, err
	}
	if limit <= 0 {
		limit = 50
	}
	if !repo.HasHead() {
		return &HistoryResult{Versions: []Version{}}, nil
	}
	out, err := repo.LogOneline(limit)
	if err != nil {
		return nil, err
	}
	var versions []Version
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, "\t", 3)
		if len(parts) < 3 {
			continue
		}
		sha, ts, msg := parts[0], parts[1], parts[2]
		reason := "snapshot"
		if strings.Contains(msg, "initial snapshot") {
			reason = "init"
		} else if strings.Contains(msg, "restore") {
			reason = "restore"
		} else if strings.Contains(msg, "pre-restore") {
			reason = "pre_restore_snapshot"
		} else {
			reason = "manual_snapshot"
		}
		count, _ := countUserFacingChanges(repo, sha)
		versions = append(versions, Version{
			ID:          VersionIDFromSHA(sha),
			Timestamp:   ts,
			Reason:      reason,
			Message:     strings.TrimPrefix(msg, "kn: "),
			ChangedDocs: count,
			InternalSHA: sha,
		})
	}
	if versions == nil {
		versions = []Version{}
	}
	return &HistoryResult{Versions: versions}, nil
}

func countUserFacingChanges(repo *Repo, sha string) (int, error) {
	out, err := repo.run("diff-tree", "--root", "--no-commit-id", "--name-only", "-r", sha)
	if err != nil {
		return 0, err
	}
	n := 0
	seenFolders := map[string]struct{}{}
	for _, l := range strings.Split(out, "\n") {
		l = strings.TrimSpace(l)
		if l == "" || workspace.IsInternalPath(l) {
			continue
		}
		if workspace.IsKeepMarker(l) {
			f := workspace.FolderFromKeep(l)
			if _, ok := seenFolders[f]; ok {
				continue
			}
			seenFolders[f] = struct{}{}
			n++
			continue
		}
		n++
	}
	return n, nil
}

func Restore(cwd, versionID string) (*RestoreResult, error) {
	_, cfg, st, repo, err := loadWS(cwd)
	if err != nil {
		return nil, err
	}
	short, err := SHAFromVersionID(versionID)
	if err != nil {
		return nil, err
	}
	full, err := repo.ResolveSHA(short)
	if err != nil {
		return nil, fmt.Errorf("version not found: %s", versionID)
	}
	targetVID := VersionIDFromSHA(full)

	// Snapshot current first
	if err := EnsureEmptyFolderMarkers(cfg.RootPath); err != nil {
		return nil, err
	}
	preMsg := "kn: pre-restore snapshot"
	preSHA, err := repo.CommitTree(preMsg)
	if err != nil {
		return nil, err
	}
	preVID := VersionIDFromSHA(preSHA)

	// Restore files from target version into work tree
	if err := repo.CheckoutPaths(full); err != nil {
		return nil, err
	}
	// Remove files that exist now but not in target (except .kn)
	if err := removeExtraFiles(cfg.RootPath, repo, full); err != nil {
		return nil, err
	}

	restoreMsg := fmt.Sprintf("kn: restore %s", targetVID)
	newSHA, err := repo.CommitTree(restoreMsg)
	if err != nil {
		return nil, err
	}
	newVID := VersionIDFromSHA(newSHA)

	// Never rewind sync baseline
	baseline := st.SyncBaselineID
	st.LatestVersionID = newVID
	st.SyncBaselineID = baseline
	if err := workspace.SaveState(cfg.RootPath, st); err != nil {
		return nil, err
	}

	return &RestoreResult{
		RestoredVersionID: targetVID,
		NewVersionID:      newVID,
		PreRestoreSnapID:  preVID,
		Message:           fmt.Sprintf("restored %s as new version %s (sync baseline unchanged)", targetVID, newVID),
		SyncBaselineID:    baseline,
	}, nil
}

func removeExtraFiles(root string, repo *Repo, targetSHA string) error {
	out, err := repo.run("ls-tree", "-r", "--name-only", targetSHA)
	if err != nil {
		return err
	}
	want := map[string]struct{}{}
	for _, l := range strings.Split(out, "\n") {
		l = strings.TrimSpace(l)
		if l != "" {
			want[l] = struct{}{}
		}
	}
	return filepath.Walk(root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		rel, err := workspace.RelToRoot(root, path)
		if err != nil {
			return err
		}
		if rel == "." {
			return nil
		}
		if rel == workspace.DirName || strings.HasPrefix(rel, workspace.DirName+"/") {
			if info.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		if info.IsDir() {
			return nil
		}
		if _, ok := want[rel]; !ok {
			return os.Remove(path)
		}
		return nil
	})
}
