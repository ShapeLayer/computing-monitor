# Architecture

## Components

### Collector

The collector runs on the monitored host and is responsible for:

- sampling the process table
- serving process detail
- tracking ephemeral operator notes keyed to the current process instance
- launching managed processes
- capturing managed stdout and stderr into a bounded in-memory log buffer
- performing local control actions such as terminate

This MVP uses a single-process Rust service with an HTTP API.

### Server

The server is the public API boundary for the web client. In this MVP it proxies requests to a single collector. The server is where authentication, authorization, and multi-host routing will eventually live.

### Web

The web app is built with Svelte and shows the current process list, process detail, capability flags, and managed-process logs. The UI is capability-aware: unsupported features are presented as such rather than implied.

## Capability model

Each process detail response includes capability flags and the current process-instance note, if present:

- `canTerminate`
- `hasManagedLogs`
- `openFilesSupported`
- `gpuSupported`

For unmanaged processes, `hasManagedLogs` is false. For this MVP, `openFilesSupported` and `gpuSupported` are always false.

## Managed execution contract

Managed commands are launched by the collector. Their stdout and stderr are captured through redirected pipes. This is the only guaranteed output capture path in the current system.

Captured lines are appended both to a bounded in-memory tail and to a per-run JSONL file under `collector/data/managed-logs`. Live viewers subscribe through an SSE endpoint exposed by the collector and proxied by the server.

## Known limits

- CPU usage comes from sampled process data and may read as zero immediately after service startup.
- Log capture is line-oriented and may not preserve partial binary writes cleanly.
- Existing unmanaged processes do not expose retroactive stdout/stderr.
