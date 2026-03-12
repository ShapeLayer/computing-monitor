# API

## Server API

### `GET /health`

Returns server and collector health.

### `GET /api/capabilities`

Returns product-wide feature flags for this MVP.

### `GET /api/processes`

Returns current process summaries.

### `GET /api/processes/:pid`

Returns one process detail with capability flags.

### `POST /api/processes/:pid/note`

Creates, updates, or clears a note for the current process instance. An empty `note` clears it.

Request body:

```json
{
  "note": "Investigating unexpected CPU spike"
}
```

### `POST /api/processes/:pid/actions/terminate`

Attempts to terminate a process by PID.

### `GET /api/managed-processes`

Returns managed runs launched by the collector.

### `POST /api/managed-processes`

Launches a new managed command.

Request body:

```json
{
  "command": "python",
  "args": ["-m", "http.server"],
  "cwd": "/tmp"
}
```

### `GET /api/managed-processes/:runId/logs?offset=0&limit=200`

Returns log lines captured for the managed run.

### `GET /api/managed-processes/:runId/logs/stream`

Streams managed log lines over Server-Sent Events.

### `POST /api/managed-processes/:runId/actions/terminate`

Terminates a managed run.
