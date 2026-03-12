const collectorBaseUrl = process.env.COLLECTOR_BASE_URL ?? "http://127.0.0.1:7001";

async function collectorRequest<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${collectorBaseUrl}${path}`, {
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {})
    },
    ...init
  });

  if (!response.ok) {
    const message = await response.text();
    throw new Error(message || `collector request failed: ${response.status}`);
  }

  return (await response.json()) as T;
}

export const collectorClient = {
  health: () => collectorRequest<{ status: string; managedRuns: number }>("/health"),
  listProcesses: () => collectorRequest<unknown[]>("/api/processes"),
  getProcess: (pid: string) => collectorRequest<unknown>(`/api/processes/${pid}`),
  updateProcessNote: (pid: string, note: string) =>
    collectorRequest<unknown>(`/api/processes/${pid}/note`, {
      method: "POST",
      body: JSON.stringify({ note })
    }),
  terminateProcess: (pid: string) =>
    collectorRequest<{ success: boolean }>(`/api/processes/${pid}/actions/terminate`, {
      method: "POST"
    }),
  listManagedProcesses: () => collectorRequest<unknown[]>("/api/managed-processes"),
  startManagedProcess: (body: Record<string, unknown>) =>
    collectorRequest<unknown>("/api/managed-processes", {
      method: "POST",
      body: JSON.stringify(body)
    }),
  getManagedLogs: (runId: string, offset = 0, limit = 200) =>
    collectorRequest<unknown[]>(`/api/managed-processes/${runId}/logs?offset=${offset}&limit=${limit}`),
  streamManagedLogs: async (runId: string) =>
    fetch(`${collectorBaseUrl}/api/managed-processes/${runId}/logs/stream`, {
      headers: {
        Accept: "text/event-stream"
      }
    }),
  terminateManagedProcess: (runId: string) =>
    collectorRequest<{ success: boolean }>(`/api/managed-processes/${runId}/actions/terminate`, {
      method: "POST"
    })
};
