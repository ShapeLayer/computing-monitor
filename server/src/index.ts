import cors from "cors";
import express from "express";

import { managedProcessesRouter } from "./routes/managedProcesses.js";
import { processesRouter } from "./routes/processes.js";
import { collectorClient } from "./services/collectorClient.js";

const app = express();
const port = Number(process.env.PORT ?? 17700);
const webApiBaseUrl = process.env.WEB_API_BASE_URL?.trim() || null;

app.use(cors());
app.use(express.json());

app.get("/health", async (_request, response, next) => {
  try {
    const collector = await collectorClient.health();
    response.json({
      status: "ok",
      collector
    });
  } catch (error) {
    next(error);
  }
});

app.get("/api/capabilities", (_request, response) => {
  response.json({
    managedStdoutCapture: true,
    managedStderrCapture: true,
    unmanagedStdoutCapture: false,
    openFiles: false,
    perProcessGpu: false,
    multiHost: false
  });
});

app.get("/api/client-config", (_request, response) => {
  response.json({
    apiBaseUrl: webApiBaseUrl
  });
});

app.use("/api/processes", processesRouter);
app.use("/api/managed-processes", managedProcessesRouter);

app.use((error: Error, _request: express.Request, response: express.Response, _next: express.NextFunction) => {
  response.status(502).json({
    error: error.message
  });
});

app.listen(port, "0.0.0.0", () => {
  console.log(`server listening on http://0.0.0.0:${port}`);
});
