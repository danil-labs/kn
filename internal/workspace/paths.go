package workspace

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// IsInternalPath reports whether rel is kn-internal and excluded from the document set.
func IsInternalPath(rel string) bool {
	rel = filepath.ToSlash(rel)
	rel = strings.TrimPrefix(rel, "./")
	if rel == DirName || strings.HasPrefix(rel, DirName+"/") {
		return true
	}
	base := filepath.Base(rel)
	if base == KeepMarker {
		return true
	}
	if base == ".gitignore" {
		return true
	}
	return false
}

// IsKeepMarker reports whether the path is an empty-folder marker.
func IsKeepMarker(rel string) bool {
	return filepath.Base(filepath.ToSlash(rel)) == KeepMarker
}

// FolderFromKeep returns the folder path for a .knkeep file (slash-separated, trailing slash).
func FolderFromKeep(rel string) string {
	rel = filepath.ToSlash(rel)
	dir := filepath.ToSlash(filepath.Dir(rel))
	if dir == "." || dir == "" {
		return "./"
	}
	if !strings.HasSuffix(dir, "/") {
		dir += "/"
	}
	return dir
}

// SafeJoin joins root and rel and ensures the result stays under root.
func SafeJoin(root, rel string) (string, error) {
	if rel == "" {
		return "", fmt.Errorf("empty path")
	}
	clean := filepath.Clean(rel)
	if filepath.IsAbs(clean) {
		return "", fmt.Errorf("absolute paths not allowed: %s", rel)
	}
	if clean == ".." || strings.HasPrefix(clean, ".."+string(os.PathSeparator)) {
		return "", fmt.Errorf("path traversal blocked: %s", rel)
	}
	full := filepath.Join(root, clean)
	absRoot, err := filepath.Abs(root)
	if err != nil {
		return "", err
	}
	absFull, err := filepath.Abs(full)
	if err != nil {
		return "", err
	}
	sep := string(os.PathSeparator)
	if absFull != absRoot && !strings.HasPrefix(absFull, absRoot+sep) {
		return "", fmt.Errorf("path escapes workspace root: %s", rel)
	}
	return absFull, nil
}

// ValidateSymlinkTarget ensures a symlink under root does not point outside root.
func ValidateSymlinkTarget(root, linkPath string) error {
	target, err := os.Readlink(linkPath)
	if err != nil {
		return err
	}
	var resolved string
	if filepath.IsAbs(target) {
		resolved = filepath.Clean(target)
	} else {
		resolved = filepath.Clean(filepath.Join(filepath.Dir(linkPath), target))
	}
	absRoot, err := filepath.Abs(root)
	if err != nil {
		return err
	}
	absResolved, err := filepath.Abs(resolved)
	if err != nil {
		return err
	}
	sep := string(os.PathSeparator)
	if absResolved != absRoot && !strings.HasPrefix(absResolved, absRoot+sep) {
		return fmt.Errorf("outbound symlink blocked: %s -> %s", linkPath, target)
	}
	return nil
}

// RelToRoot returns a slash path relative to root.
func RelToRoot(root, full string) (string, error) {
	absRoot, err := filepath.Abs(root)
	if err != nil {
		return "", err
	}
	absFull, err := filepath.Abs(full)
	if err != nil {
		return "", err
	}
	rel, err := filepath.Rel(absRoot, absFull)
	if err != nil {
		return "", err
	}
	if strings.HasPrefix(rel, "..") {
		return "", fmt.Errorf("path outside root")
	}
	return filepath.ToSlash(rel), nil
}
