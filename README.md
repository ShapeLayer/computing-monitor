# Computing Monitor

Computing Monitor is a remote process monitoring MVP split into three parts:

- `collector`: a host-local Rust service that samples processes, launches managed commands, and captures stdout/stderr.
- `server`: a Node.js API that fronts the collector for a web client.
- `web`: a Svelte dashboard for process browsing, process detail, and managed command execution.

## MVP scope

This initial implementation supports:

- process list with CPU, memory, command line, and executable path
- per-process ephemeral notes scoped to the current process instance and current monitor session
- process detail with basic capability flags and termination
- process termination requests
- managed process launch with stdout/stderr capture
- managed process log persistence on disk
- managed process log browsing and SSE streaming
- compact web-application UI with sortable process table, per-row actions, and detail inspector

This initial implementation does not yet support:

- per-process GPU usage
- open file enumeration
- best-effort discovery from journald, Event Log, or macOS unified logging
- multi-host enrollment and authentication

## Run

### Collector

```bash
cd collector
cargo run
```

### Server

```bash
cd server
npm install
npm run dev
```

### Web

```bash
cd web
npm install
npm run dev
```

Default ports:

- collector: `7001`
- server: `17700`
- web: `5173`

Environment variables:

- `COLLECTOR_BASE_URL` (server): collector endpoint (default: `http://127.0.0.1:7001`)
- `WEB_API_BASE_URL` (server): optional API base URL override for web clients, returned by `GET /api/client-config`
  - The web client automatically ignores loopback-only overrides (`localhost`/`127.0.0.1`/`::1`) when the page is opened from a remote host.
- `VITE_API_BASE_URL` (web): optional build-time API base URL override
  - If this is a loopback URL and the page is opened from a non-loopback hostname, the app falls back to the current browser host.
- `VITE_API_PORT` (web): default API port used with current browser host when neither `VITE_API_BASE_URL` nor `/api/client-config` provide a usable API base URL (default: `17700`)
