# Knowledge (`kn`) — MVP PRD v2 (phase 2 summary)

Date: 2026-09-08 · Status: phase 2 local core delivery note

This document summarizes **phase 2: local core** for the Knowledge CLI. Full product intent remains: a bidirectional adapter/normalizer/sync layer for local knowledge workspaces and cloud document systems (Drive / SharePoint). Cloud sync is **out of scope** for this phase.

## Product stance

- Cloud is optional; a disconnected workspace stays useful for status, diff, history, snapshot, and restore.
- Git is an encapsulated internal version engine — not the product interface.
- Primary promise for later phases: **never silently overwrite unknown remote changes**.

## Phase 2 commands

| Command | Behavior |
|---|---|
| `kn init` | Workspace ID, initial snapshot, idempotent, no cloud |
| `kn status` | Local changes, pending sync (empty), conflicts (empty), capabilities, last remote observation (`unknown`) |
| `kn status --refresh` | Structured unsupported, exit 3 |
| `kn diff` | Changes since latest snapshot; `--base` vs sync baseline when present |
| `kn history` | Version IDs, timestamps, reasons, changed-document counts |
| `kn restore <version>` | Snapshot current → restore files+folders → new version; never rewinds sync baseline; never remote write |
| `kn snapshot [-m msg]` | Optional named checkpoint |
| `kn connect` / `pull` / `push` | Structured unsupported, exit 3 |

All commands support `--json` with envelope `{schema_version, operation_id, status, data, conflicts, errors}`.

Exit codes: `0` ok, `1` operational/partial, `2` conflict, `3` invalid/unsupported.

## Architecture (delivered)

- Separate Git dir: `~/.kn/repos/<workspace-id>/`
- Knowledge folder as `GIT_WORK_TREE`
- `.kn/` for nonsecret state; document set excludes `.kn`
- No kn-owned `.git` required inside the knowledge folder; existing user `.git` undisturbed
- Version IDs abstract internal commit SHAs (`v_<12-hex>`)
- Empty folders tracked via `.knkeep` markers (presented as folder changes in UX)
- Path traversal and outbound symlinks blocked

## Acceptance checklist (phase 2)

1. Offline init / status / diff / snapshot / history / restore without OAuth or visible kn `.git` in the workspace
2. `kn init` idempotent with workspace ID + initial snapshot
3. `--json` envelope on all commands
4. Exit codes as specified
5. Remote commands return clear unsupported (exit 3) — no fake cloud
6. Tests cover init → edit → status/diff → snapshot → history → restore
7. README uses documents / changes / versions terminology

## Explicitly not in this PR

- Google Drive / Microsoft SharePoint providers
- OAuth, pull/push reconciliation, remote identity maps
- `status --refresh` remote observations
- Approvals, real-time sync, background watchers

Those remain north-star items for later phases.
