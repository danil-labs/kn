package workspace

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/google/uuid"
)

const (
	DirName    = ".kn"
	ConfigFile = "config.json"
	StateFile  = "state.json"
	KeepMarker = ".knkeep"
	SchemaVer  = 1
)

type Config struct {
	SchemaVersion int    `json:"schema_version"`
	WorkspaceID   string `json:"workspace_id"`
	RootPath      string `json:"root_path"`
	CreatedAt     string `json:"created_at"`
}

type State struct {
	SchemaVersion      int     `json:"schema_version"`
	LatestVersionID    string  `json:"latest_version_id,omitempty"`
	SyncBaselineID     string  `json:"sync_baseline_id,omitempty"`
	LastRemoteObserved *string `json:"last_remote_observed,omitempty"`
	RemoteFreshness    string  `json:"remote_freshness"` // unknown | stale | fresh
	Capabilities       Caps    `json:"capabilities"`
}

type Caps struct {
	LocalVersioning bool `json:"local_versioning"`
	CloudConnect    bool `json:"cloud_connect"`
	Pull            bool `json:"pull"`
	Push            bool `json:"push"`
	RemoteRefresh   bool `json:"remote_refresh"`
}

func KnDir(root string) string {
	return filepath.Join(root, DirName)
}

func ConfigPath(root string) string {
	return filepath.Join(KnDir(root), ConfigFile)
}

func StatePath(root string) string {
	return filepath.Join(KnDir(root), StateFile)
}

func HomeKnDir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".kn"), nil
}

func RepoDir(workspaceID string) (string, error) {
	base, err := HomeKnDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(base, "repos", workspaceID), nil
}

func FindRoot(start string) (string, error) {
	abs, err := filepath.Abs(start)
	if err != nil {
		return "", err
	}
	cur := abs
	for {
		if _, err := os.Stat(ConfigPath(cur)); err == nil {
			return cur, nil
		}
		parent := filepath.Dir(cur)
		if parent == cur {
			return "", errors.New("not a kn workspace (no .kn/config.json found); run kn init")
		}
		cur = parent
	}
}

func LoadConfig(root string) (*Config, error) {
	b, err := os.ReadFile(ConfigPath(root))
	if err != nil {
		return nil, err
	}
	var c Config
	if err := json.Unmarshal(b, &c); err != nil {
		return nil, err
	}
	return &c, nil
}

func LoadState(root string) (*State, error) {
	b, err := os.ReadFile(StatePath(root))
	if err != nil {
		return nil, err
	}
	var s State
	if err := json.Unmarshal(b, &s); err != nil {
		return nil, err
	}
	return &s, nil
}

func SaveConfig(root string, c *Config) error {
	if err := os.MkdirAll(KnDir(root), 0o755); err != nil {
		return err
	}
	b, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(ConfigPath(root), append(b, '\n'), 0o644)
}

func SaveState(root string, s *State) error {
	if err := os.MkdirAll(KnDir(root), 0o755); err != nil {
		return err
	}
	b, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(StatePath(root), append(b, '\n'), 0o644)
}

func DefaultState() *State {
	return &State{
		SchemaVersion:   SchemaVer,
		RemoteFreshness: "unknown",
		Capabilities: Caps{
			LocalVersioning: true,
			CloudConnect:    false,
			Pull:            false,
			Push:            false,
			RemoteRefresh:   false,
		},
	}
}

func NewWorkspaceID() string {
	return uuid.New().String()
}

func EnsureInitialized(root string) (*Config, *State, bool, error) {
	abs, err := filepath.Abs(root)
	if err != nil {
		return nil, nil, false, err
	}
	if _, err := os.Stat(ConfigPath(abs)); err == nil {
		cfg, err := LoadConfig(abs)
		if err != nil {
			return nil, nil, false, err
		}
		st, err := LoadState(abs)
		if err != nil {
			st = DefaultState()
		}
		return cfg, st, true, nil
	}
	id := NewWorkspaceID()
	cfg := &Config{
		SchemaVersion: SchemaVer,
		WorkspaceID:   id,
		RootPath:      abs,
		CreatedAt:     time.Now().UTC().Format(time.RFC3339),
	}
	st := DefaultState()
	if err := SaveConfig(abs, cfg); err != nil {
		return nil, nil, false, err
	}
	if err := SaveState(abs, st); err != nil {
		return nil, nil, false, err
	}
	return cfg, st, false, nil
}

func FormatInitSummary(cfg *Config, already bool) string {
	if already {
		return fmt.Sprintf("workspace already initialized (id=%s)", cfg.WorkspaceID)
	}
	return fmt.Sprintf("initialized workspace id=%s", cfg.WorkspaceID)
}
