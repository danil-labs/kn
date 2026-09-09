package engine

import (
	"os"
	"path/filepath"
	"strings"

	"github.com/soydanil/kn/internal/workspace"
)

// EnsureEmptyFolderMarkers walks the work tree and places .knkeep in empty dirs
// (excluding .kn). Also removes stale markers from non-empty dirs.
func EnsureEmptyFolderMarkers(root string) error {
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
		if workspace.IsInternalPath(rel) && info.IsDir() && filepath.Base(rel) == workspace.DirName {
			return filepath.SkipDir
		}
		if !info.IsDir() {
			return nil
		}
		// Check outbound symlink dirs
		if info.Mode()&os.ModeSymlink != 0 {
			if err := workspace.ValidateSymlinkTarget(root, path); err != nil {
				return err
			}
		}
		entries, err := os.ReadDir(path)
		if err != nil {
			return err
		}
		// Count non-marker children
		realKids := 0
		hasMarker := false
		for _, e := range entries {
			if e.Name() == workspace.KeepMarker {
				hasMarker = true
				continue
			}
			realKids++
		}
		markerPath := filepath.Join(path, workspace.KeepMarker)
		if realKids == 0 {
			if !hasMarker {
				if err := os.WriteFile(markerPath, []byte(""), 0o644); err != nil {
					return err
				}
			}
		} else if hasMarker {
			_ = os.Remove(markerPath)
		}
		return nil
	})
}

// ScanDangerousSymlinks fails if any symlink points outside the workspace.
func ScanDangerousSymlinks(root string) error {
	return filepath.Walk(root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		rel, err := workspace.RelToRoot(root, path)
		if err != nil {
			return err
		}
		if rel == workspace.DirName || strings.HasPrefix(rel, workspace.DirName+"/") {
			if info.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		if info.Mode()&os.ModeSymlink != 0 {
			return workspace.ValidateSymlinkTarget(root, path)
		}
		return nil
	})
}
