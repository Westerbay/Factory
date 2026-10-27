import {
  workerRequestSchema,
  type WorkerResponse,
} from "@workspace/contracts/desktop"
import { DesktopHost, PersistenceCloseError } from "./host.js"
const MAX_IN_FLIGHT = 64
const RESERVED_CONTROL_OPERATIONS = 4
export interface WorkerPort {
  on(event: "message", listener: (event: { data: unknown }) => void): void
  postMessage(value: WorkerResponse): void
}
export function serveWorker(
  port: WorkerPort,
  generation: string,
  createHost: (
    publish: (message: WorkerResponse) => void
  ) => Promise<DesktopHost>
) {
  let host: DesktopHost | undefined
  let accepting = true
  const active = new Set<string>()
  const send = (value: WorkerResponse) => port.postMessage(value)
  const status = (
    state: "starting" | "ready" | "unavailable" | "closing" | "closed",
    message: string
  ) => send({ type: "status", generation, status: { state, message } })
  const observe = (message: WorkerResponse) => {
    if (
      message.type === "event" &&
      message.event.type === "diagnostic" &&
      message.event.code === "persistence"
    ) {
      accepting = false
      status("unavailable", message.event.message)
    }
    send(message)
  }
  const initialized = createHost(observe)
    .then(async (created) => {
      host = created
      await host.initialize()
      status(
        "ready",
        "Owned database ready. Provider readiness is checked separately."
      )
    })
    .catch(async () => {
      accepting = false
      try {
        await host?.close()
      } catch {
        /* Startup remains visibly unavailable if cleanup also fails. */
      }
      status(
        "unavailable",
        "Runtime startup failed. Run pnpm db:status; provision or migrate explicitly, then restart Factory."
      )
    })
  status(
    "starting",
    "Checking owned persistence and recovering interrupted work…"
  )
  port.on("message", (event) => {
    const parsed = workerRequestSchema.safeParse(event.data)
    if (!parsed.success || parsed.data.generation !== generation) return
    const request = parsed.data
    if (active.has(request.requestId)) return
    const control =
      request.type === "close" ||
      (request.type === "command" && request.command.type === "cancel-run")
    const capacity = control
      ? MAX_IN_FLIGHT + RESERVED_CONTROL_OPERATIONS
      : MAX_IN_FLIGHT
    if (active.size >= capacity) {
      send({
        type: "result",
        generation,
        requestId: request.requestId,
        result: {
          type: "error",
          code: "unavailable",
          message: "Too many operations are pending.",
        },
      })
      return
    }
    active.add(request.requestId)
    // Independent operations are essential: one multi-question reply can wait for another.
    void handle().finally(() => active.delete(request.requestId))
    async function handle() {
      await initialized
      try {
        if (request.type === "close") {
          accepting = false
          status("closing", "Stopping all owned execution resources…")
          await host?.close()
          status("closed", "Runtime and persistence closed.")
          send({
            type: "result",
            generation,
            requestId: request.requestId,
            result: { type: "acknowledged" },
          })
          return
        }
        if (!accepting || !host) {
          send({
            type: "result",
            generation,
            requestId: request.requestId,
            result: {
              type: "error",
              code: "unavailable",
              message:
                "Runtime is unavailable. Restart after checking persistence.",
            },
          })
          return
        }
        if (request.type === "workspace") {
          host.registerWorkspace(request.workspace, request.path)
          send({
            type: "result",
            generation,
            requestId: request.requestId,
            result: { type: "acknowledged" },
          })
          return
        }
        send({
          type: "result",
          generation,
          requestId: request.requestId,
          result: await host.command(request.command),
        })
      } catch (error) {
        const message =
          error instanceof PersistenceCloseError
            ? error.message
            : "Runtime operation failed; cleanup may be incomplete. You may explicitly retry closing after checking owned resources. Ordinary work remains disabled."
        if (request.type === "close") status("unavailable", message)
        send({
          type: "result",
          generation,
          requestId: request.requestId,
          result: {
            type: "error",
            code:
              error instanceof PersistenceCloseError
                ? "persistence"
                : "runtime",
            message,
          },
        })
      }
    }
  })
}
