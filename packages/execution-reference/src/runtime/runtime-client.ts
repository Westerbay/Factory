import { randomUUID } from "node:crypto"
import {
  workerResponseSchema,
  type WorkerRequest,
  type HostStatus,
  type BridgeEvent,
} from "@workspace/contracts/desktop"
import type { DesktopCommand, DesktopResult } from "@workspace/contracts"
const OPERATION_TIMEOUT_MS = 30_000
const MAX_PENDING = 64
const RESERVED_CONTROL_OPERATIONS = 4
export interface RuntimeProcess {
  postMessage(value: WorkerRequest): void
  on(event: "message", listener: (value: unknown) => void): unknown
  on(event: "exit", listener: (code: number) => void): unknown
}
interface PendingOperation {
  control: boolean
  closing: boolean
  resolve: (result: DesktopResult) => void
  timer?: ReturnType<typeof setTimeout>
}
export class RuntimeClient {
  readonly generation = randomUUID()
  private pending = new Map<string, PendingOperation>()
  private process?: RuntimeProcess
  private lost = false
  private exited = false
  private statusValue: HostStatus = {
    state: "starting",
    message: "Starting isolated execution host…",
  }
  constructor(private publish: (event: BridgeEvent) => void) {}
  attach(process: RuntimeProcess) {
    if (this.process) throw new Error("A runtime is already owned")
    this.process = process
    process.on("message", (message) => this.receive(message))
    process.on("exit", () => {
      this.exited = true
      if (this.statusValue.state !== "closed")
        this.fail(
          "Execution host exited. Owned provider cleanup is unconfirmed. Restart after checking local resources; persisted work will be marked interrupted."
        )
    })
  }
  status(): HostStatus {
    return this.statusValue
  }
  private update(status: HostStatus) {
    this.statusValue = status
    this.publish({ type: "host-status", status })
  }
  private receive(value: unknown) {
    const parsed = workerResponseSchema.safeParse(value)
    if (!parsed.success) {
      this.fail(
        "Execution host sent an invalid message. Cleanup is unconfirmed."
      )
      return
    }
    const message = parsed.data
    if (message.generation !== this.generation) return
    if (message.type === "status") {
      if (!this.lost) this.update(message.status)
    } else if (message.type === "event") {
      if (!this.lost) this.publish(message.event)
    } else {
      const operation = this.pending.get(message.requestId)
      if (!operation || (this.lost && !operation.control)) return
      if (operation.closing && message.result.type === "acknowledged")
        this.statusValue = {
          state: "closed",
          message: "Owned runtime cleanup and persistence close confirmed.",
        }
      clearTimeout(operation.timer)
      this.pending.delete(message.requestId)
      operation.resolve(message.result)
    }
  }
  fail(message: string) {
    this.lost = true
    this.update({ state: "unavailable", message })
    for (const operation of this.pending.values()) {
      clearTimeout(operation.timer)
      operation.resolve({ type: "error", code: "unavailable", message })
    }
    this.pending.clear()
  }
  command(command: DesktopCommand) {
    const response =
      command.type === "answer-question" || command.type === "answer-permission"
    return this.send(
      {
        type: "command",
        generation: this.generation,
        requestId: randomUUID(),
        command,
      },
      response
    )
  }
  registerWorkspace(workspace: { id: string; name: string }, path: string) {
    return this.send({
      type: "workspace",
      generation: this.generation,
      requestId: randomUUID(),
      workspace,
      path,
    })
  }
  close() {
    return this.send({
      type: "close",
      generation: this.generation,
      requestId: randomUUID(),
    })
  }
  private send(
    request: WorkerRequest,
    waitsForHuman = false
  ): Promise<DesktopResult> {
    const process = this.process
    const control =
      request.type === "close" ||
      (request.type === "command" && request.command.type === "cancel-run")
    const capacity = control
      ? MAX_PENDING + RESERVED_CONTROL_OPERATIONS
      : MAX_PENDING
    if (
      (this.lost && !control) ||
      this.exited ||
      !process ||
      this.pending.size >= capacity
    )
      return Promise.resolve({
        type: "error",
        code: "unavailable",
        message: "Runtime unavailable or too many operations pending.",
      })
    return new Promise((resolve) => {
      // Human multi-question replies have no deadline. Capacity and worker death bound their lifetime.
      let timer: ReturnType<typeof setTimeout> | undefined
      if (!waitsForHuman)
        timer = setTimeout(
          () =>
            this.fail(
              "Execution host timed out. The action outcome and owned cleanup are unconfirmed. Do not automatically retry; restart after checking resources."
            ),
          OPERATION_TIMEOUT_MS
        )
      this.pending.set(request.requestId, {
        resolve,
        timer,
        control,
        closing: request.type === "close",
      })
      try {
        process.postMessage(request)
      } catch {
        this.fail(
          "Execution host communication failed. Cleanup is unconfirmed."
        )
      }
    })
  }
}
