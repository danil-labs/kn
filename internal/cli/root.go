package cli

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"

	"github.com/soydanil/kn/internal/api"
	"github.com/soydanil/kn/internal/engine"
)

// Run parses args and executes a command. Returns process exit code.
func Run(args []string, stdout, stderr io.Writer) int {
	if len(args) == 0 {
		return printUsage(stdout, stderr)
	}

	jsonOut := false
	filtered := make([]string, 0, len(args))
	for _, a := range args {
		if a == "--json" {
			jsonOut = true
			continue
		}
		if a == "--help" || a == "-h" {
			return printUsage(stdout, stderr)
		}
		filtered = append(filtered, a)
	}
	if len(filtered) == 0 {
		return printUsage(stdout, stderr)
	}

	cmd := filtered[0]
	rest := filtered[1:]

	cwd, err := os.Getwd()
	if err != nil {
		return emit(stdout, stderr, jsonOut, api.Fail("cwd_error", err.Error(), "fix working directory", true))
	}

	switch cmd {
	case "init":
		return cmdInit(cwd, jsonOut, stdout, stderr)
	case "status":
		return cmdStatus(cwd, rest, jsonOut, stdout, stderr)
	case "diff":
		return cmdDiff(cwd, rest, jsonOut, stdout, stderr)
	case "history":
		return cmdHistory(cwd, rest, jsonOut, stdout, stderr)
	case "snapshot":
		return cmdSnapshot(cwd, rest, jsonOut, stdout, stderr)
	case "restore":
		return cmdRestore(cwd, rest, jsonOut, stdout, stderr)
	case "connect":
		return cmdUnsupported("connect", "CLOUD_CONNECT_UNSUPPORTED",
			"cloud connect is not implemented in phase 2 (local core only)",
			"use local commands (init/status/diff/snapshot/history/restore); cloud arrives in a later phase",
			jsonOut, stdout, stderr)
	case "pull":
		return cmdUnsupported("pull", "PULL_UNSUPPORTED",
			"pull is not implemented in phase 2 (local core only)",
			"stay offline or wait for cloud sync phase",
			jsonOut, stdout, stderr)
	case "push":
		return cmdUnsupported("push", "PUSH_UNSUPPORTED",
			"push is not implemented in phase 2 (local core only)",
			"stay offline or wait for cloud sync phase",
			jsonOut, stdout, stderr)
	case "version", "--version":
		data := map[string]string{"version": "0.2.0-local", "phase": "2-local-core"}
		if jsonOut {
			return emit(stdout, stderr, true, api.NewEnvelope(api.StatusOK, data))
		}
		fmt.Fprintln(stdout, "kn 0.2.0-local (phase 2 local core)")
		return api.ExitOK
	case "help":
		return printUsage(stdout, stderr)
	default:
		env := api.Unsupported("UNKNOWN_COMMAND", fmt.Sprintf("unknown command: %s", cmd), "run kn --help")
		return emit(stdout, stderr, jsonOut, env)
	}
}

func printUsage(stdout, stderr io.Writer) int {
	fmt.Fprint(stdout, `kn — Knowledge CLI (phase 2: local core)

Usage:
  kn init                 Initialize workspace (offline, idempotent)
  kn status               Local changes, capabilities, remote freshness
  kn status --refresh     Unsupported (exit 3) until cloud phase
  kn diff [--base]        Changes since latest snapshot (or sync baseline)
  kn history              List versions
  kn snapshot [-m msg]    Create a named checkpoint
  kn restore <version>    Restore documents/folders to a prior version
  kn connect ...          Unsupported (exit 3)
  kn pull | kn push       Unsupported (exit 3)

All commands accept --json for the stable agent envelope.
`)
	return api.ExitOK
}

func emit(stdout, stderr io.Writer, jsonOut bool, env *api.Envelope) int {
	if env.Status != api.StatusOK {
		for _, e := range env.Errors {
			api.Diagnof("%s: %s", e.Code, e.Message)
			fmt.Fprintf(stderr, "error: %s (%s)\n", e.Message, e.Code)
		}
	}
	if jsonOut {
		if err := env.WriteJSON(stdout); err != nil {
			fmt.Fprintf(stderr, "failed to write json: %v\n", err)
			return api.ExitPartial
		}
	} else {
		writeHuman(stdout, env)
	}
	return api.ExitFor(env.Status)
}

func writeHuman(w io.Writer, env *api.Envelope) {
	if env.Status != api.StatusOK && env.Status != api.StatusPartial {
		for _, e := range env.Errors {
			fmt.Fprintf(w, "%s\n", e.Message)
		}
		return
	}
	b, err := json.MarshalIndent(env.Data, "", "  ")
	if err != nil {
		fmt.Fprintf(w, "%v\n", env.Data)
		return
	}
	fmt.Fprintln(w, string(b))
}

func cmdInit(cwd string, jsonOut bool, stdout, stderr io.Writer) int {
	res, err := engine.InitWorkspace(cwd)
	if err != nil {
		return emit(stdout, stderr, jsonOut, api.Fail("INIT_FAILED", err.Error(), "fix permissions and retry kn init", true))
	}
	return emit(stdout, stderr, jsonOut, api.NewEnvelope(api.StatusOK, res))
}

func cmdStatus(cwd string, rest []string, jsonOut bool, stdout, stderr io.Writer) int {
	refresh := false
	for _, a := range rest {
		if a == "--refresh" {
			refresh = true
		}
	}
	if refresh {
		return cmdUnsupported("status --refresh", "REMOTE_REFRESH_UNSUPPORTED",
			"remote status refresh is not implemented in phase 2 (local core only)",
			"omit --refresh for local status; cloud refresh arrives later",
			jsonOut, stdout, stderr)
	}
	res, err := engine.Status(cwd)
	if err != nil {
		code := "STATUS_FAILED"
		next := "run kn init in the knowledge folder"
		if strings.Contains(err.Error(), "not a kn workspace") {
			code = "NOT_A_WORKSPACE"
		}
		return emit(stdout, stderr, jsonOut, api.Fail(code, err.Error(), next, false))
	}
	return emit(stdout, stderr, jsonOut, api.NewEnvelope(api.StatusOK, res))
}

func cmdDiff(cwd string, rest []string, jsonOut bool, stdout, stderr io.Writer) int {
	useBase := false
	includePatch := false
	for _, a := range rest {
		switch a {
		case "--base":
			useBase = true
		case "--patch":
			includePatch = true
		case "--remote":
			return cmdUnsupported("diff --remote", "REMOTE_DIFF_UNSUPPORTED",
				"remote diff is not implemented in phase 2",
				"use kn diff or kn diff --base when a sync baseline exists",
				jsonOut, stdout, stderr)
		}
	}
	res, err := engine.Diff(cwd, useBase, includePatch)
	if err != nil {
		code := "DIFF_FAILED"
		if strings.Contains(err.Error(), "no sync baseline") {
			env := api.Unsupported("NO_SYNC_BASELINE", err.Error(), "sync baseline is set after cloud connect/pull in a later phase")
			return emit(stdout, stderr, jsonOut, env)
		}
		return emit(stdout, stderr, jsonOut, api.Fail(code, err.Error(), "run kn init first", false))
	}
	return emit(stdout, stderr, jsonOut, api.NewEnvelope(api.StatusOK, res))
}

func cmdHistory(cwd string, rest []string, jsonOut bool, stdout, stderr io.Writer) int {
	res, err := engine.History(cwd, 50)
	if err != nil {
		return emit(stdout, stderr, jsonOut, api.Fail("HISTORY_FAILED", err.Error(), "run kn init first", false))
	}
	return emit(stdout, stderr, jsonOut, api.NewEnvelope(api.StatusOK, res))
}

func cmdSnapshot(cwd string, rest []string, jsonOut bool, stdout, stderr io.Writer) int {
	msg := ""
	for i := 0; i < len(rest); i++ {
		if rest[i] == "-m" || rest[i] == "--message" {
			if i+1 < len(rest) {
				msg = rest[i+1]
				i++
			}
		}
	}
	res, err := engine.Snapshot(cwd, msg)
	if err != nil {
		return emit(stdout, stderr, jsonOut, api.Fail("SNAPSHOT_FAILED", err.Error(), "fix workspace state and retry", true))
	}
	return emit(stdout, stderr, jsonOut, api.NewEnvelope(api.StatusOK, res))
}

func cmdRestore(cwd string, rest []string, jsonOut bool, stdout, stderr io.Writer) int {
	if len(rest) < 1 {
		return emit(stdout, stderr, jsonOut, api.Unsupported("MISSING_VERSION", "kn restore requires a version id", "run kn history, then kn restore <version>"))
	}
	vid := rest[0]
	res, err := engine.Restore(cwd, vid)
	if err != nil {
		code := "RESTORE_FAILED"
		if strings.Contains(err.Error(), "not found") {
			code = "VERSION_NOT_FOUND"
			return emit(stdout, stderr, jsonOut, api.Unsupported(code, err.Error(), "run kn history to list version ids"))
		}
		return emit(stdout, stderr, jsonOut, api.Fail(code, err.Error(), "retry restore or snapshot current work first", true))
	}
	return emit(stdout, stderr, jsonOut, api.NewEnvelope(api.StatusOK, res))
}

func cmdUnsupported(op, code, msg, next string, jsonOut bool, stdout, stderr io.Writer) int {
	_ = op
	return emit(stdout, stderr, jsonOut, api.Unsupported(code, msg, next))
}
