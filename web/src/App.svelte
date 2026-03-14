<script lang="ts">
  import {
    ArrowUpDown,
    Info,
    NotebookPen,
    RefreshCw,
    Search,
    SquareTerminal,
    Terminal,
    Trash2,
    X
  } from "lucide-svelte";
  import { onDestroy, onMount, tick } from "svelte";

  type ProcessSummary = {
    pid: number;
    parentPid?: number;
    instanceId: string;
    name: string;
    cpuPercent: number;
    memoryBytes: number;
    virtualMemoryBytes: number;
    status: string;
    startedAt?: string;
    commandLine: string;
    executablePath?: string;
    note?: string;
  };

  type ProcessCapabilities = {
    canTerminate: boolean;
    hasManagedLogs: boolean;
    openFilesSupported: boolean;
    gpuSupported: boolean;
  };

  type ProcessDetail = {
    summary: ProcessSummary;
    capabilities: ProcessCapabilities;
  };

  type ManagedProcessSummary = {
    runId: string;
    pid?: number;
    command: string;
    args: string[];
    cwd?: string;
    startedAt: string;
    status: string;
  };

  type ManagedLogLine = {
    offset: number;
    timestamp: string;
    stream: string;
    message: string;
  };

  type GlobalCapabilities = {
    managedStdoutCapture: boolean;
    managedStderrCapture: boolean;
    unmanagedStdoutCapture: boolean;
    openFiles: boolean;
    perProcessGpu: boolean;
    multiHost: boolean;
  };

  type ProcessNoteResponse = {
    instanceId: string;
    note: string;
    updatedAt: string;
  } | null;

  type ClientConfig = {
    apiBaseUrl?: string | null;
  };

  type ProcessSortKey = "name" | "pid" | "cpuPercent" | "memoryBytes" | "status" | "note";
  type OverlayMode = "detail" | "note" | "logs" | null;

  const defaultApiPort = import.meta.env.VITE_API_PORT ?? "17700";
  const loopbackHosts = new Set(["localhost", "127.0.0.1", "::1"]);

  function isLoopbackHost(hostname: string) {
    return loopbackHosts.has(hostname.toLowerCase().replace(/^\[|\]$/g, ""));
  }

  const isBrowserLoopback = isLoopbackHost(window.location.hostname);

  function normalizeApiBaseUrl(candidate: string | undefined | null): string | null {
    if (!candidate?.trim()) {
      return null;
    }

    try {
      const parsed = new URL(candidate, window.location.href);
      if (isLoopbackHost(parsed.hostname) && !isBrowserLoopback) {
        return null;
      }

      return parsed.origin;
    } catch {
      return null;
    }
  }

  function buildDefaultApiBaseUrl() {
    const { protocol, hostname } = window.location;
    return `${protocol}//${hostname}:${defaultApiPort}`;
  }

  function getInitialApiBaseUrl() {
    const envBaseUrl = normalizeApiBaseUrl(import.meta.env.VITE_API_BASE_URL);
    return envBaseUrl ?? buildDefaultApiBaseUrl();
  }

  let apiBaseUrl = getInitialApiBaseUrl();

  let processes: ProcessSummary[] = [];
  let managedRuns: ManagedProcessSummary[] = [];
  let capabilities: GlobalCapabilities | null = null;
  let selectedPid: number | null = null;
  let selectedProcess: ProcessDetail | null = null;
  let selectedRunId = "";
  let overlayMode: OverlayMode = null;
  let logs: ManagedLogLine[] = [];
  let command = "/bin/sh";
  let args = "-lc while true; do date; sleep 2; done";
  let cwd = "";
  let processSearch = "";
  let sortKey: ProcessSortKey = "cpuPercent";
  let sortDirection: "asc" | "desc" = "desc";
  let filteredProcesses: ProcessSummary[] = [];
  let noteDraft = "";
  let noteDraftInstanceId = "";
  let error = "";
  let processPollHandle: number | undefined;
  let detailPollHandle: number | undefined;
  let eventSource: EventSource | null = null;
  let noteEditor: HTMLTextAreaElement | null = null;

  function humanMb(bytes: number) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }

  function humanStart(value?: string) {
    if (!value) {
      return "unknown";
    }

    return new Date(value).toLocaleString();
  }

  function isSortActive(key: ProcessSortKey) {
    return sortKey === key;
  }

  function compareProcess(left: ProcessSummary, right: ProcessSummary) {
    const direction = sortDirection === "asc" ? 1 : -1;

    if (sortKey === "name" || sortKey === "status" || sortKey === "note") {
      const leftValue = String(sortKey === "note" ? left.note ?? "" : left[sortKey]).toLowerCase();
      const rightValue = String(sortKey === "note" ? right.note ?? "" : right[sortKey]).toLowerCase();
      return leftValue.localeCompare(rightValue) * direction;
    }

    return ((left[sortKey] as number) - (right[sortKey] as number)) * direction;
  }

  function toggleSort(nextKey: ProcessSortKey) {
    if (sortKey === nextKey) {
      sortDirection = sortDirection === "asc" ? "desc" : "asc";
      return;
    }

    sortKey = nextKey;
    sortDirection = nextKey === "name" || nextKey === "status" || nextKey === "note" ? "asc" : "desc";
  }

  function focusNoteEditor() {
    noteEditor?.focus();
  }

  function syncNoteDraft(detail: ProcessDetail | null) {
    const nextInstanceId = detail?.summary.instanceId ?? "";
    if (nextInstanceId !== noteDraftInstanceId) {
      noteDraftInstanceId = nextInstanceId;
      noteDraft = detail?.summary.note ?? "";
    }
  }

  function findManagedRunByPid(pid: number | null) {
    if (pid === null) {
      return undefined;
    }

    return managedRuns.find((run) => run.pid === pid);
  }

  async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
    const response = await fetch(`${apiBaseUrl}${path}`, init);
    if (!response.ok) {
      throw new Error(await response.text());
    }
    return (await response.json()) as T;
  }

  async function loadClientConfig() {
    try {
      const response = await fetch(`${apiBaseUrl}/api/client-config`);
      if (!response.ok) {
        return;
      }

      const config = (await response.json()) as ClientConfig;
      const normalized = normalizeApiBaseUrl(config.apiBaseUrl);
      if (normalized) {
        apiBaseUrl = normalized;
      }
    } catch {
      // Keep default API base URL when runtime client config is unavailable.
    }
  }

  async function refreshOverview() {
    try {
      const [nextProcesses, nextManagedRuns, nextCapabilities] = await Promise.all([
        fetchJson<ProcessSummary[]>("/api/processes"),
        fetchJson<ManagedProcessSummary[]>("/api/managed-processes"),
        fetchJson<GlobalCapabilities>("/api/capabilities")
      ]);

      processes = nextProcesses;
      managedRuns = nextManagedRuns;
      capabilities = nextCapabilities;

      if (selectedPid === null || !nextProcesses.some((process) => process.pid === selectedPid)) {
        selectedPid = nextProcesses[0]?.pid ?? null;
      }

      if (!selectedRunId || !nextManagedRuns.some((run) => run.runId === selectedRunId)) {
        selectedRunId = nextManagedRuns[0]?.runId ?? "";
        connectLogStream();
      }

      error = "";
    } catch (nextError) {
      error = nextError instanceof Error ? nextError.message : "failed to fetch overview";
    }
  }

  async function refreshSelectedProcess() {
    if (selectedPid === null) {
      selectedProcess = null;
      return;
    }

    try {
      selectedProcess = await fetchJson<ProcessDetail>(`/api/processes/${selectedPid}`);
      syncNoteDraft(selectedProcess);
      error = "";
    } catch (nextError) {
      error = nextError instanceof Error ? nextError.message : "failed to fetch process detail";
    }
  }

  function connectLogStream() {
    if (eventSource) {
      eventSource.close();
      eventSource = null;
    }

    logs = [];

    if (!selectedRunId) {
      return;
    }

    eventSource = new EventSource(`${apiBaseUrl}/api/managed-processes/${selectedRunId}/logs/stream`);
    eventSource.addEventListener("log", (event) => {
      const payload = JSON.parse((event as MessageEvent<string>).data) as ManagedLogLine;
      logs = [...logs, payload].slice(-500);
    });
    eventSource.onerror = () => {
      error = "log stream disconnected";
    };
  }

  async function selectProcess(pid: number) {
    selectedPid = pid;
    await refreshSelectedProcess();
  }

  async function openProcessOverlay(pid: number, mode: Exclude<OverlayMode, null>) {
    await selectProcess(pid);
    overlayMode = mode;

    if (mode === "note") {
      await tick();
      focusNoteEditor();
    }
  }

  function closeOverlay() {
    overlayMode = null;
  }

  function selectRun(runId: string) {
    selectedRunId = runId;
    connectLogStream();
  }

  async function launchManagedProcess() {
    try {
      const createdRun = await fetchJson<ManagedProcessSummary>("/api/managed-processes", {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify({
          command,
          args: args.split(" ").filter(Boolean),
          cwd: cwd || undefined
        })
      });

      selectedRunId = createdRun.runId;
      await refreshOverview();
      connectLogStream();
    } catch (nextError) {
      error = nextError instanceof Error ? nextError.message : "failed to launch managed process";
    }
  }

  async function terminateSelectedProcess() {
    if (selectedPid === null) {
      return;
    }

    try {
      await fetchJson(`/api/processes/${selectedPid}/actions/terminate`, {
        method: "POST"
      });
      await refreshOverview();
      await refreshSelectedProcess();
    } catch (nextError) {
      error = nextError instanceof Error ? nextError.message : "failed to terminate process";
    }
  }

  async function terminateSelectedRun() {
    if (!selectedRunId) {
      return;
    }

    try {
      await fetchJson(`/api/managed-processes/${selectedRunId}/actions/terminate`, {
        method: "POST"
      });
      await refreshOverview();
    } catch (nextError) {
      error = nextError instanceof Error ? nextError.message : "failed to terminate managed run";
    }
  }

  async function saveNote() {
    if (selectedPid === null) {
      return;
    }

    try {
      const result = await fetchJson<ProcessNoteResponse>(`/api/processes/${selectedPid}/note`, {
        method: "POST",
        headers: {
          "content-type": "application/json"
        },
        body: JSON.stringify({
          note: noteDraft
        })
      });

      await refreshOverview();
      await refreshSelectedProcess();
      noteDraftInstanceId = result?.instanceId ?? selectedProcess?.summary.instanceId ?? "";
      noteDraft = result?.note ?? "";
    } catch (nextError) {
      error = nextError instanceof Error ? nextError.message : "failed to save note";
    }
  }

  async function openManagedLogsForProcess(pid: number) {
    const run = findManagedRunByPid(pid);
    if (!run) {
      error = "no managed run found for this process";
      return;
    }

    selectedPid = pid;
    selectedRunId = run.runId;
    overlayMode = "logs";
    connectLogStream();
    error = "";
  }

  function onOverlayBackdropClick(event: MouseEvent) {
    if (event.target === event.currentTarget) {
      closeOverlay();
    }
  }

  function onOverlayBackdropKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      closeOverlay();
    }
  }

  onMount(async () => {
    await loadClientConfig();
    await refreshOverview();
    if (selectedPid !== null) {
      await refreshSelectedProcess();
    }

    processPollHandle = window.setInterval(refreshOverview, 3000);
    detailPollHandle = window.setInterval(refreshSelectedProcess, 3000);
  });

  onDestroy(() => {
    if (processPollHandle) {
      window.clearInterval(processPollHandle);
    }
    if (detailPollHandle) {
      window.clearInterval(detailPollHandle);
    }
    if (eventSource) {
      eventSource.close();
    }
  });

  $: filteredProcesses = processes
    .filter((process) => {
      const query = processSearch.trim().toLowerCase();
      if (!query) {
        return true;
      }

      return [
        process.name,
        String(process.pid),
        process.commandLine,
        process.note ?? ""
      ].some((value) => value.toLowerCase().includes(query));
    })
    .slice()
    .sort(compareProcess);
</script>

<svelte:head>
  <title>Computing Monitor</title>
</svelte:head>

<div class="shell">
  <section class="workspace-toolbar panel">
    <div class="toolbar-group toolbar-primary">
      <label class="search-field">
        <Search size={16} />
        <input bind:value={processSearch} class="search-input" placeholder="Search process name, pid, command, or note" />
      </label>
      <button class="secondary-button icon-text-button" on:click={refreshOverview} title="Refresh process table and managed runs" aria-label="Refresh process table and managed runs">
        <RefreshCw size={16} />
        <span>Refresh</span>
      </button>
      <button class="primary-button icon-text-button" on:click={() => { overlayMode = "logs"; if (selectedRunId) connectLogStream(); }} title="Open managed process launcher and logs" aria-label="Open managed process launcher and logs">
        <SquareTerminal size={16} />
        <span>Managed</span>
      </button>
    </div>
    <div class="toolbar-group toolbar-status">
      <span class="status-chip">Processes {filteredProcesses.length}</span>
      <span class="status-chip">Managed stdout {capabilities?.managedStdoutCapture ? "on" : "off"}</span>
      <span class="status-chip">Managed stderr {capabilities?.managedStderrCapture ? "on" : "off"}</span>
      <span class="status-chip muted">Unmanaged stdout {capabilities?.unmanagedStdoutCapture ? "best-effort" : "off"}</span>
    </div>
  </section>

  {#if error}
    <div class="error-banner">{error}</div>
  {/if}

  <main class="table-only-layout">
    <section class="panel layout-stack process-panel">
      <div class="panel-header">
        <h2>Processes</h2>
        <span>{filteredProcesses.length} visible</span>
      </div>

      <div class="table-shell">
        <table class="process-table">
          <thead>
            <tr>
              <th><button class:active-sort={isSortActive("name")} class="column-button" on:click={() => toggleSort("name")} title="Sort by process name"><span>Name</span><ArrowUpDown size={14} /></button></th>
              <th><button class:active-sort={isSortActive("pid")} class="column-button" on:click={() => toggleSort("pid")} title="Sort by process id"><span>PID</span><ArrowUpDown size={14} /></button></th>
              <th><button class:active-sort={isSortActive("cpuPercent")} class="column-button" on:click={() => toggleSort("cpuPercent")} title="Sort by CPU usage"><span>CPU</span><ArrowUpDown size={14} /></button></th>
              <th><button class:active-sort={isSortActive("memoryBytes")} class="column-button" on:click={() => toggleSort("memoryBytes")} title="Sort by memory usage"><span>Memory</span><ArrowUpDown size={14} /></button></th>
              <th><button class:active-sort={isSortActive("status")} class="column-button" on:click={() => toggleSort("status")} title="Sort by process status"><span>Status</span><ArrowUpDown size={14} /></button></th>
              <th><button class:active-sort={isSortActive("note")} class="column-button" on:click={() => toggleSort("note")} title="Sort by note presence or content"><span>Note</span><ArrowUpDown size={14} /></button></th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {#each filteredProcesses as process (process.instanceId)}
              <tr class:selected-row={selectedPid === process.pid && overlayMode !== null}>
                <td>
                  <button class="row-link" on:click={() => openProcessOverlay(process.pid, "detail")} title="Open process detail overlay">
                    <strong>{process.name}</strong>
                  </button>
                </td>
                <td>{process.pid}</td>
                <td>{process.cpuPercent.toFixed(1)}%</td>
                <td>{humanMb(process.memoryBytes)}</td>
                <td>{process.status}</td>
                <td class:has-note={Boolean(process.note)}>{process.note ?? "-"}</td>
                <td>
                  <div class="action-icons">
                    <button class="icon-button" on:click={() => openProcessOverlay(process.pid, "detail")} title="Show detailed information" aria-label="Show detailed information">
                      <Info size={15} />
                    </button>
                    <button class="icon-button" on:click={() => openProcessOverlay(process.pid, "note")} title="Add or edit an instance note" aria-label="Add or edit an instance note">
                      <NotebookPen size={15} />
                    </button>
                    <button class="icon-button" disabled={!findManagedRunByPid(process.pid)} on:click={() => openManagedLogsForProcess(process.pid)} title="Open managed logs and runs overlay" aria-label="Open managed logs and runs overlay">
                      <Terminal size={15} />
                    </button>
                    <button class="icon-button danger" on:click={() => { selectedPid = process.pid; void terminateSelectedProcess(); }} title="Terminate process" aria-label="Terminate process">
                      <Trash2 size={15} />
                    </button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </section>
  </main>

  {#if overlayMode !== null}
    <div
      class="overlay-backdrop"
      role="button"
      tabindex="0"
      aria-label="Close overlay"
      on:click={onOverlayBackdropClick}
      on:keydown={onOverlayBackdropKeydown}
    >
      <div
        class={`overlay-panel ${overlayMode === "logs" ? "overlay-wide" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={overlayMode === "detail" ? "Process detail" : overlayMode === "note" ? "Process note" : "Managed runs and logs"}
      >
        <div class="overlay-header">
          <div>
            <h2>
              {#if overlayMode === "detail"}
                Process Detail
              {:else if overlayMode === "note"}
                Instance Note
              {:else}
                Managed Runs And Logs
              {/if}
            </h2>
            <p class="overlay-subtitle">
              {#if overlayMode === "logs"}
                Launch and inspect managed processes without leaving the table.
              {:else}
                {selectedProcess?.summary.name ?? "No process selected"}
              {/if}
            </p>
          </div>
          <button class="overlay-close" on:click={closeOverlay} title="Close overlay" aria-label="Close overlay">
            <X size={16} />
            <span>Close</span>
          </button>
        </div>

        {#if overlayMode === "detail" && selectedProcess}
          <div class="overlay-body overlay-stack">
            <div class="action-row">
              <span class="status-chip">PID {selectedProcess.summary.pid}</span>
              <button class="danger-button icon-text-button" on:click={terminateSelectedProcess} disabled={!selectedProcess.capabilities.canTerminate}>
                <Trash2 size={16} />
                <span>Terminate</span>
              </button>
            </div>

            <div class="detail-grid">
              <div class="detail-tile"><strong>Status</strong><p>{selectedProcess.summary.status}</p></div>
              <div class="detail-tile"><strong>Started</strong><p>{humanStart(selectedProcess.summary.startedAt)}</p></div>
              <div class="detail-tile"><strong>CPU</strong><p>{selectedProcess.summary.cpuPercent.toFixed(1)}%</p></div>
              <div class="detail-tile"><strong>Memory</strong><p>{humanMb(selectedProcess.summary.memoryBytes)}</p></div>
              <div class="detail-tile"><strong>Virtual Memory</strong><p>{humanMb(selectedProcess.summary.virtualMemoryBytes)}</p></div>
              <div class="detail-tile"><strong>Instance</strong><p>{selectedProcess.summary.instanceId}</p></div>
            </div>

            <div class="capability-list">
              <div class="capability-item">
                <strong>Managed Logs</strong>
                <div class:off={!selectedProcess.capabilities.hasManagedLogs} class:on={selectedProcess.capabilities.hasManagedLogs} class="capability-state">
                  {selectedProcess.capabilities.hasManagedLogs ? "available" : "unmanaged process"}
                </div>
              </div>
              <div class="capability-item">
                <strong>Open Files</strong>
                <div class:off={!selectedProcess.capabilities.openFilesSupported} class:on={selectedProcess.capabilities.openFilesSupported} class="capability-state">
                  {selectedProcess.capabilities.openFilesSupported ? "supported" : "planned"}
                </div>
              </div>
              <div class="capability-item">
                <strong>Per-process GPU</strong>
                <div class:off={!selectedProcess.capabilities.gpuSupported} class:on={selectedProcess.capabilities.gpuSupported} class="capability-state">
                  {selectedProcess.capabilities.gpuSupported ? "supported" : "planned"}
                </div>
              </div>
              <div class="capability-item">
                <strong>Terminate</strong>
                <div class:off={!selectedProcess.capabilities.canTerminate} class:on={selectedProcess.capabilities.canTerminate} class="capability-state">
                  {selectedProcess.capabilities.canTerminate ? "allowed" : "not allowed"}
                </div>
              </div>
            </div>

            <div class="detail-tile">
              <strong>Executable Path</strong>
              <p>{selectedProcess.summary.executablePath ?? "unknown"}</p>
            </div>

            <div class="detail-tile">
              <strong>Command</strong>
              <p>{selectedProcess.summary.commandLine || "no command line"}</p>
            </div>
          </div>
        {:else if overlayMode === "note" && selectedProcess}
          <div class="overlay-body overlay-stack">
            <div class="detail-tile">
              <strong>Target Instance</strong>
              <p>{selectedProcess.summary.name} · PID {selectedProcess.summary.pid} · {selectedProcess.summary.instanceId}</p>
            </div>

            <div class="detail-tile note-editor-tile">
              <div class="panel-header compact-header">
                <h2>Instance Note</h2>
                <span>Ephemeral</span>
              </div>
              <textarea bind:this={noteEditor} bind:value={noteDraft} class="note-editor" placeholder="Add an operator note for this process instance only"></textarea>
              <div class="action-row">
                <button class="secondary-button" on:click={() => (noteDraft = "")}>Clear draft</button>
                <button class="primary-button icon-text-button" on:click={saveNote}>
                  <NotebookPen size={16} />
                  <span>Save note</span>
                </button>
              </div>
            </div>
          </div>
        {:else if overlayMode === "logs"}
          <div class="overlay-body overlay-logs-grid">
            <section class="overlay-column overlay-stack">
              <div class="panel-header compact-header">
                <h2>Launch Managed Process</h2>
                <span>{managedRuns.length} tracked</span>
              </div>

              <div class="launcher">
                <label>
                  Command
                  <input bind:value={command} />
                </label>
                <label>
                  Args
                  <input bind:value={args} />
                </label>
                <label>
                  CWD
                  <input bind:value={cwd} placeholder="optional working directory" />
                </label>
                <button class="primary-button icon-text-button" on:click={launchManagedProcess}>
                  <Terminal size={16} />
                  <span>Launch managed process</span>
                </button>
              </div>

              <div class="action-row">
                <button class="secondary-button icon-text-button" on:click={refreshOverview}>
                  <RefreshCw size={16} />
                  <span>Refresh</span>
                </button>
                <button class="danger-button icon-text-button" on:click={terminateSelectedRun} disabled={!selectedRunId}>
                  <Trash2 size={16} />
                  <span>Terminate run</span>
                </button>
              </div>

              <div class="managed-list">
                {#each managedRuns as run (run.runId)}
                  <button class:selected={selectedRunId === run.runId} class="managed-item" on:click={() => selectRun(run.runId)}>
                    <div>
                      <strong>{run.command}</strong>
                      <p>{run.args.join(" ")}</p>
                    </div>
                    <span>{run.status}</span>
                  </button>
                {/each}
              </div>
            </section>

            <section class="overlay-column overlay-stack">
              <div class="panel-header compact-header">
                <h2>Captured Output</h2>
                <span>{selectedRunId || "select a run"}</span>
              </div>

              <div class="log-viewer overlay-log-viewer">
                {#if logs.length === 0}
                  <p>No captured lines yet.</p>
                {/if}

                {#each logs as line (`${line.offset}-${line.timestamp}`)}
                  <div class={`log-line ${line.stream}`}>
                    <span class="log-meta">[{line.stream}]</span>
                    <span>{line.message}</span>
                  </div>
                {/each}
              </div>
            </section>
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>
