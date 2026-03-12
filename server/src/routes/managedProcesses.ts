import { Router } from "express";

import { collectorClient } from "../services/collectorClient.js";

export const managedProcessesRouter = Router();

managedProcessesRouter.get("/", async (_request, response, next) => {
  try {
    response.json(await collectorClient.listManagedProcesses());
  } catch (error) {
    next(error);
  }
});

managedProcessesRouter.post("/", async (request, response, next) => {
  try {
    response.json(await collectorClient.startManagedProcess(request.body));
  } catch (error) {
    next(error);
  }
});

managedProcessesRouter.get("/:runId/logs", async (request, response, next) => {
  try {
    const offset = Number(request.query.offset ?? 0);
    const limit = Number(request.query.limit ?? 200);
    response.json(await collectorClient.getManagedLogs(request.params.runId, offset, limit));
  } catch (error) {
    next(error);
  }
});

managedProcessesRouter.get("/:runId/logs/stream", async (request, response, next) => {
  try {
    const upstream = await collectorClient.streamManagedLogs(request.params.runId);

    if (!upstream.ok || !upstream.body) {
      response.status(upstream.status).end();
      return;
    }

    response.setHeader("Content-Type", "text/event-stream");
    response.setHeader("Cache-Control", "no-cache");
    response.setHeader("Connection", "keep-alive");
    response.flushHeaders();

    const reader = upstream.body.getReader();

    const pump = async () => {
      while (true) {
        const { done, value } = await reader.read();
        if (done) {
          response.end();
          break;
        }
        response.write(Buffer.from(value));
      }
    };

    request.on("close", () => {
      reader.cancel().catch(() => undefined);
      response.end();
    });

    void pump();
  } catch (error) {
    next(error);
  }
});

managedProcessesRouter.post("/:runId/actions/terminate", async (request, response, next) => {
  try {
    response.json(await collectorClient.terminateManagedProcess(request.params.runId));
  } catch (error) {
    next(error);
  }
});
