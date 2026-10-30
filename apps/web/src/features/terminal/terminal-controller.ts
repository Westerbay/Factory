import { ApiFailure } from "../../lib/api/client"
import type { SessionController } from "../../lib/auth/session-controller"
import {
  type ClientMessage,
  openTerminalSocket,
  requestAttachment,
  serverMessageSchema,
  terminalProblem,
} from "./api"

type TerminalState = {
  status:
    | "idle"
    | "opening"
    | "connected"
    | "disconnected"
    | "conflict"
    | "error"
  message: string
}
type Display = {
  write: (data: Uint8Array, done: () => void) => void
  reset: () => void
}
type Dependencies = {
  session: Pick<
    SessionController,
    "getSnapshot" | "subscribe" | "authorizationFailed"
  >
  request: typeof requestAttachment
  openSocket: typeof openTerminalSocket
  display: Display
}
const INPUT_QUEUE_BYTES = 64 * 1024
const OUTPUT_QUEUE_BYTES = 1024 * 1024
const OUTPUT_CHUNK_BYTES = 16 * 1024
const INPUT_BATCH_MS = 20
const encoder = new TextEncoder()

/** Owns only the browser attachment. Closing it never sends an environment Stop. */
export class TerminalController {
  private state: TerminalState = {
    status: "idle",
    message: "Open your terminal to attach to its persistent shell.",
  }
  private listeners = new Set<() => void>()
  private generation = 0
  private request?: AbortController
  private socket?: WebSocket
  private unsubscribe?: () => void
  private inputTimer?: ReturnType<typeof setTimeout>
  private openingTimer?: ReturnType<typeof setTimeout>
  private input = ""
  private pendingOutput = 0
  private userId?: string
  private size = { cols: 80, rows: 24 }

  constructor(private dependencies: Dependencies) {}
  getSnapshot = () => this.state
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }
  private set(state: TerminalState) {
    this.state = state
    for (const listener of this.listeners) listener()
  }
  private closeTransport() {
    this.generation++
    this.request?.abort()
    clearTimeout(this.inputTimer)
    clearTimeout(this.openingTimer)
    this.inputTimer = undefined
    this.input = ""
    this.pendingOutput = 0
    const socket = this.socket
    this.socket = undefined
    if (socket) {
      socket.onopen = socket.onmessage = socket.onerror = socket.onclose = null
      socket.close()
    }
  }
  private sessionChanged = () => {
    const state = this.dependencies.session.getSnapshot()
    if (state.status !== "active" || state.session.user.id !== this.userId) {
      this.closeTransport()
      this.dependencies.display.reset()
      this.set({
        status: "disconnected",
        message:
          "Your session changed. Reconnect after your access has been checked.",
      })
    }
  }
  async connect(takeOver = false) {
    this.closeTransport()
    this.unsubscribe ??= this.dependencies.session.subscribe(
      this.sessionChanged
    )
    const auth = this.dependencies.session.getSnapshot()
    if (auth.status !== "active") {
      this.sessionChanged()
      return
    }
    this.userId = auth.session.user.id
    const generation = this.generation
    this.dependencies.display.reset()
    this.set({ status: "opening", message: "Opening your terminal…" })
    this.request = new AbortController()
    try {
      const ticket = await this.dependencies.request(
        takeOver,
        auth.session.csrfToken,
        this.request.signal
      )
      if (generation !== this.generation) return
      const socket = this.dependencies.openSocket()
      this.socket = socket
      socket.binaryType = "arraybuffer"
      this.openingTimer = setTimeout(
        () =>
          this.fail(
            "The terminal did not open in time. Check your environment, then reconnect."
          ),
        60_000
      )
      socket.onopen = () => {
        if (generation !== this.generation) return
        this.send({ type: "authenticate", ticket: ticket.ticket })
        socket.onopen = null
      }
      socket.onmessage = (event: MessageEvent<unknown>) => {
        if (generation !== this.generation) return
        this.receive(event.data, generation)
      }
      socket.onerror = () => {
        if (generation === this.generation)
          this.fail(
            "The terminal connection failed. Check your environment, then reconnect."
          )
      }
      socket.onclose = () => {
        if (generation === this.generation) this.disconnect()
      }
    } catch (error) {
      if (generation !== this.generation) return
      if (error instanceof ApiFailure && error.status === 409) {
        this.closeTransport()
        this.set({ status: "conflict", message: terminalProblem("conflict") })
      } else if (
        error instanceof ApiFailure &&
        [401, 403].includes(error.status)
      ) {
        this.dependencies.session.authorizationFailed()
        this.sessionChanged()
      } else this.fail(terminalProblem("unavailable"))
    }
  }
  private send(message: ClientMessage) {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return
    if (this.socket.bufferedAmount > INPUT_QUEUE_BYTES) {
      this.fail(
        "The connection is too slow to send more input. Reconnect; unsent keystrokes will not be replayed."
      )
      return
    }
    try {
      this.socket.send(JSON.stringify(message))
    } catch {
      this.fail("The terminal connection closed. Reconnect to continue.")
    }
  }
  private receive(data: unknown, generation: number) {
    if (data instanceof ArrayBuffer) {
      if (
        this.state.status !== "connected" ||
        !data.byteLength ||
        data.byteLength > OUTPUT_CHUNK_BYTES ||
        this.pendingOutput + data.byteLength > OUTPUT_QUEUE_BYTES
      ) {
        this.fail(terminalProblem("invalid_input"))
        return
      }
      this.pendingOutput += data.byteLength
      this.dependencies.display.write(new Uint8Array(data), () => {
        if (generation !== this.generation) return
        this.pendingOutput -= data.byteLength
        this.send({ type: "ack", bytes: data.byteLength })
      })
      return
    }
    if (typeof data !== "string" || data.length > 2048) {
      this.fail(terminalProblem("invalid_input"))
      return
    }
    let message
    try {
      message = serverMessageSchema.parse(JSON.parse(data))
    } catch {
      this.fail(terminalProblem("invalid_input"))
      return
    }
    if (message.type === "ready") {
      if (this.state.status !== "opening") {
        this.fail(terminalProblem("invalid_input"))
        return
      }
      clearTimeout(this.openingTimer)
      this.set({
        status: "connected",
        message: "Connected. Closing this view leaves your command running.",
      })
      this.send({ type: "resize", ...this.size })
    } else if (message.type === "exit") {
      this.closeTransport()
      this.set({
        status: "disconnected",
        message: "The shell exited. Reconnect to open a shell again.",
      })
    } else {
      this.closeTransport()
      this.set({
        status: ["conflict", "cleanup_uncertain"].includes(message.code)
          ? "conflict"
          : "error",
        message: terminalProblem(message.code),
      })
      if (message.code === "unauthorized") {
        this.dependencies.display.reset()
        this.dependencies.session.authorizationFailed()
      }
    }
  }
  inputText = (data: string) => {
    if (this.state.status !== "connected") return
    if (
      data.length > INPUT_QUEUE_BYTES ||
      encoder.encode(this.input + data).byteLength > INPUT_QUEUE_BYTES
    ) {
      this.set({
        ...this.state,
        message: "This paste is too large. Paste fewer than 64 KiB at a time.",
      })
      return
    }
    this.input += data
    this.inputTimer ??= setTimeout(this.flushInput, INPUT_BATCH_MS)
  }
  private flushInput = () => {
    this.inputTimer = undefined
    const input = this.input
    this.input = ""
    // 4096 UTF-16 units fit in a 16-KiB UTF-8 message. Keep surrogate pairs intact.
    for (
      let start = 0;
      start < input.length && this.state.status === "connected";
    ) {
      let end = Math.min(start + 4096, input.length)
      if (end < input.length && /[\uD800-\uDBFF]/.test(input[end - 1]!)) end--
      this.send({ type: "input", data: input.slice(start, end) })
      start = end
    }
  }
  resize = (cols: number, rows: number) => {
    this.size = {
      cols: Math.max(2, Math.min(500, cols)),
      rows: Math.max(1, Math.min(200, rows)),
    }
    if (this.state.status === "connected")
      this.send({ type: "resize", ...this.size })
  }
  disconnect = () => {
    this.closeTransport()
    this.set({
      status: "disconnected",
      message:
        "Disconnected from the display. Your shell can keep running; reconnect to return.",
    })
  }
  private fail(message: string) {
    this.closeTransport()
    this.set({ status: "error", message })
  }
  dispose() {
    this.closeTransport()
    this.unsubscribe?.()
    this.unsubscribe = undefined
    this.dependencies.display.reset()
  }
}
