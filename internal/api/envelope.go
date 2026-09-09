package api

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"time"

	"github.com/google/uuid"
)

const SchemaVersion = "1.0"

// Exit codes per PRD.
const (
	ExitOK          = 0
	ExitPartial     = 1
	ExitConflict    = 2
	ExitUnsupported = 3
)

type Status string

const (
	StatusOK          Status = "ok"
	StatusError       Status = "error"
	StatusUnsupported Status = "unsupported"
	StatusConflict    Status = "conflict"
	StatusPartial     Status = "partial"
)

type Envelope struct {
	SchemaVersion string      `json:"schema_version"`
	OperationID   string      `json:"operation_id"`
	Status        Status      `json:"status"`
	Data          interface{} `json:"data"`
	Conflicts     []Conflict  `json:"conflicts"`
	Errors        []ErrorItem `json:"errors"`
}

type Conflict struct {
	ID      string `json:"id,omitempty"`
	Path    string `json:"path,omitempty"`
	Message string `json:"message"`
}

type ErrorItem struct {
	Code                string `json:"code"`
	Message             string `json:"message"`
	Retryable           bool   `json:"retryable"`
	SuggestedNextAction string `json:"suggested_next_action"`
	ItemID              string `json:"item_id,omitempty"`
}

func NewEnvelope(status Status, data interface{}) *Envelope {
	return &Envelope{
		SchemaVersion: SchemaVersion,
		OperationID:   uuid.New().String(),
		Status:        status,
		Data:          data,
		Conflicts:     []Conflict{},
		Errors:        []ErrorItem{},
	}
}

func Unsupported(code, message, next string) *Envelope {
	env := NewEnvelope(StatusUnsupported, nil)
	env.Errors = append(env.Errors, ErrorItem{
		Code:                code,
		Message:             message,
		Retryable:           false,
		SuggestedNextAction: next,
	})
	return env
}

func Fail(code, message, next string, retryable bool) *Envelope {
	env := NewEnvelope(StatusError, nil)
	env.Errors = append(env.Errors, ErrorItem{
		Code:                code,
		Message:             message,
		Retryable:           retryable,
		SuggestedNextAction: next,
	})
	return env
}

func (e *Envelope) WriteJSON(w io.Writer) error {
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	return enc.Encode(e)
}

func ExitFor(status Status) int {
	switch status {
	case StatusOK:
		return ExitOK
	case StatusPartial, StatusError:
		return ExitPartial
	case StatusConflict:
		return ExitConflict
	case StatusUnsupported:
		return ExitUnsupported
	default:
		return ExitPartial
	}
}

// Diagnof writes a diagnostic line to stderr (no secrets).
func Diagnof(format string, args ...interface{}) {
	fmt.Fprintf(os.Stderr, "[kn %s] %s\n", time.Now().UTC().Format(time.RFC3339), fmt.Sprintf(format, args...))
}
