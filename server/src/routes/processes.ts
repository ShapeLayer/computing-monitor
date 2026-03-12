import { Router } from "express";

import { collectorClient } from "../services/collectorClient.js";

export const processesRouter = Router();

processesRouter.get("/", async (_request, response, next) => {
  try {
    response.json(await collectorClient.listProcesses());
  } catch (error) {
    next(error);
  }
});

processesRouter.get("/:pid", async (request, response, next) => {
  try {
    response.json(await collectorClient.getProcess(request.params.pid));
  } catch (error) {
    next(error);
  }
});

processesRouter.post("/:pid/note", async (request, response, next) => {
  try {
    response.json(await collectorClient.updateProcessNote(request.params.pid, String(request.body?.note ?? "")));
  } catch (error) {
    next(error);
  }
});

processesRouter.post("/:pid/actions/terminate", async (request, response, next) => {
  try {
    response.json(await collectorClient.terminateProcess(request.params.pid));
  } catch (error) {
    next(error);
  }
});
