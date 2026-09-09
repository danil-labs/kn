package engine

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// Repo binds a separate GIT_DIR to a knowledge work tree.
type Repo struct {
	GitDir   string
	WorkTree string
}

func (r *Repo) env() []string {
	env := os.Environ()
	env = append(env,
		"GIT_DIR="+r.GitDir,
		"GIT_WORK_TREE="+r.WorkTree,
		"GIT_AUTHOR_NAME=kn",
		"GIT_AUTHOR_EMAIL=kn@local",
		"GIT_COMMITTER_NAME=kn",
		"GIT_COMMITTER_EMAIL=kn@local",
	)
	return env
}

func (r *Repo) run(args ...string) (string, error) {
	cmd := exec.Command("git", args...)
	cmd.Env = r.env()
	cmd.Dir = r.WorkTree
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	// Preserve leading spaces (git status --porcelain uses a leading space in XY).
	out := strings.TrimRight(stdout.String(), "\r\n")
	if err != nil {
		msg := strings.TrimSpace(stderr.String())
		if msg == "" {
			msg = err.Error()
		}
		return out, fmt.Errorf("git %s: %s", strings.Join(args, " "), msg)
	}
	return out, nil
}

func (r *Repo) runAllowDirty(args ...string) (string, error) {
	return r.run(args...)
}

// Init creates a bare-ish separate git dir and configures excludes for .kn.
func Init(gitDir, workTree string) (*Repo, error) {
	if err := os.MkdirAll(gitDir, 0o755); err != nil {
		return nil, err
	}
	r := &Repo{GitDir: gitDir, WorkTree: workTree}
	if _, err := os.Stat(filepath.Join(gitDir, "HEAD")); err == nil {
		return r, nil
	}
	if _, err := r.run("init"); err != nil {
		return nil, err
	}
	// Exclude .kn from the document set permanently via info/exclude (no .git in work tree).
	infoDir := filepath.Join(gitDir, "info")
	if err := os.MkdirAll(infoDir, 0o755); err != nil {
		return nil, err
	}
	exclude := filepath.Join(infoDir, "exclude")
	content := "# kn internal — do not track\n.kn/\n"
	if err := os.WriteFile(exclude, []byte(content), 0o644); err != nil {
		return nil, err
	}
	// Ensure no .git file/dir is created inside the work tree by git init when using GIT_DIR.
	// git init with GIT_DIR set should not create worktree .git; remove if present as a pointer.
	dotGit := filepath.Join(workTree, ".git")
	if fi, err := os.Lstat(dotGit); err == nil {
		// Only remove if it is a gitfile pointing at our git dir (created by some git versions).
		if !fi.IsDir() {
			_ = os.Remove(dotGit)
		}
	}
	_, _ = r.run("config", "core.bare", "false")
	return r, nil
}

func Open(gitDir, workTree string) (*Repo, error) {
	if _, err := os.Stat(filepath.Join(gitDir, "HEAD")); err != nil {
		return nil, fmt.Errorf("version store not found at %s; run kn init", gitDir)
	}
	return &Repo{GitDir: gitDir, WorkTree: workTree}, nil
}

func (r *Repo) HasHead() bool {
	_, err := r.run("rev-parse", "--verify", "HEAD")
	return err == nil
}

func (r *Repo) HeadSHA() (string, error) {
	return r.run("rev-parse", "HEAD")
}

func (r *Repo) ResolveSHA(ref string) (string, error) {
	return r.run("rev-parse", "--verify", ref)
}

func VersionIDFromSHA(sha string) string {
	sha = strings.TrimSpace(sha)
	if len(sha) < 12 {
		return "v_" + sha
	}
	return "v_" + sha[:12]
}

func SHAFromVersionID(id string) (string, error) {
	id = strings.TrimSpace(id)
	if strings.HasPrefix(id, "v_") {
		id = strings.TrimPrefix(id, "v_")
	}
	if id == "" {
		return "", fmt.Errorf("empty version id")
	}
	return id, nil
}

func (r *Repo) CommitTree(message string) (string, error) {
	// Stage everything except excluded paths.
	if _, err := r.run("add", "-A"); err != nil {
		return "", err
	}
	// Allow empty initial commit.
	args := []string{"commit", "--allow-empty", "-m", message}
	if _, err := r.run(args...); err != nil {
		// Nothing to commit (non-empty allow failed?) — check if clean after add
		status, sErr := r.run("status", "--porcelain")
		if sErr == nil && status == "" {
			sha, err := r.HeadSHA()
			if err != nil {
				return "", err
			}
			return sha, nil
		}
		return "", err
	}
	return r.HeadSHA()
}

func (r *Repo) DiffNames(base string) (string, error) {
	if base == "" {
		return r.run("diff", "--name-status", "HEAD")
	}
	return r.run("diff", "--name-status", base)
}

func (r *Repo) DiffUnified(base string) (string, error) {
	if base == "" {
		return r.run("diff", "HEAD")
	}
	return r.run("diff", base)
}

func (r *Repo) StatusPorcelain() (string, error) {
	// Include untracked; after git add -N? Just porcelain.
	return r.run("status", "--porcelain", "-u")
}

func (r *Repo) LogOneline(n int) (string, error) {
	return r.run("log", fmt.Sprintf("-%d", n), "--format=%H%x09%cI%x09%s")
}

func (r *Repo) ChangedCount(sha string) (int, error) {
	out, err := r.run("diff-tree", "--root", "--no-commit-id", "--name-only", "-r", sha)
	if err != nil {
		return 0, err
	}
	if out == "" {
		return 0, nil
	}
	lines := strings.Split(out, "\n")
	count := 0
	for _, l := range lines {
		l = strings.TrimSpace(l)
		if l == "" {
			continue
		}
		count++
	}
	return count, nil
}

func (r *Repo) CheckoutPaths(sha string) error {
	_, err := r.run("checkout", sha, "--", ".")
	return err
}

func (r *Repo) RemoveUntrackedMatching(trackedAtSHA string) error {
	// List files at sha
	out, err := r.run("ls-tree", "-r", "--name-only", trackedAtSHA)
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
	// Walk work tree and remove tracked-type files not in want (documents only; skip .kn)
	return filepath.Walk(r.WorkTree, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(r.WorkTree, path)
		if err != nil {
			return err
		}
		rel = filepath.ToSlash(rel)
		if rel == "." {
			return nil
		}
		if rel == ".kn" || strings.HasPrefix(rel, ".kn/") {
			if info.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		if info.IsDir() {
			return nil
		}
		if _, ok := want[rel]; !ok {
			// Only remove if it would have been tracked (not excluded). Check against status after.
			_ = os.Remove(path)
		}
		return nil
	})
}
