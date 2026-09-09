# kn

Knowledge (`kn`) is a local-first CLI for versioning knowledge documents on disk.
Collaborators may later sync with Google Drive or SharePoint; **this release is phase 2 local core only** (offline).

Git is an internal engine. You work with **documents**, **changes**, and **versions** — not staging, branches, or commits.

## Install

```bash
go install github.com/soydanil/kn/cmd/kn@latest
# or from this repo:
go build -o kn ./cmd/kn
```

Requires Go 1.22+ and `git` on `PATH`.

## Local-only workflow

```bash
cd /path/to/knowledge-folder
kn init                 # workspace id + initial version (idempotent, no cloud)
# edit documents…
kn status               # local changes, capabilities, remote freshness (unknown)
kn diff                 # changes since latest version/snapshot
kn snapshot -m "Policy update"
kn history              # version ids, timestamps, reasons, changed-doc counts
kn restore <version>    # snapshot current, restore files+folders, new version
```

All commands accept `--json` for agents.

### Concepts

| Term | Meaning |
|---|---|
| **Document** | A file in the knowledge folder (`.kn/` is excluded) |
| **Change** | Added / modified / deleted document, or empty folder add/remove |
| **Version** | A recoverable checkpoint (`v_…` id). Not a Git commit SHA in the UX |
| **Snapshot** | Optional named checkpoint (`kn snapshot -m "…"`) |
| **Sync baseline** | Last reconciled remote baseline (unused until cloud phase) |

### Layout

- Knowledge folder = your documents (work tree)
- `.kn/` inside the folder = nonsecret config/state (not part of the document set)
- `~/.kn/repos/<workspace-id>/` = separate Git directory (no `.git` required in the knowledge folder)
- Existing user `.git` directories are left untouched
- Empty folders are preserved

### Safety

- Path traversal rejected
- Outbound symlinks (pointing outside the workspace) blocked
- Diagnostics on stderr; secrets never printed
- `kn restore` never rewinds the sync baseline and never writes remotely

## JSON envelope

```json
{
  "schema_version": "1.0",
  "operation_id": "…",
  "status": "ok",
  "data": {},
  "conflicts": [],
  "errors": []
}
```

Errors include stable `code`, `retryable`, and `suggested_next_action`.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Operational failure / partial |
| 2 | Unresolved conflict |
| 3 | Invalid input or unsupported capability |

## Unsupported in this phase (exit 3)

```bash
kn connect drive|sharepoint
kn pull
kn push
kn status --refresh
kn diff --remote
```

These return a structured `unsupported` envelope. No fake cloud behavior.

## Development

```bash
go test ./...
go build -o kn ./cmd/kn
```

See [docs/PRD-MVP-v2.md](docs/PRD-MVP-v2.md) for the phase-2 acceptance summary.
