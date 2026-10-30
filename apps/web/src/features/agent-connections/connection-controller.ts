import type { Session } from "../../lib/auth/session-api"
import type { SessionController } from "../../lib/auth/session-controller"
import {
  changeConnection,
  canDisconnectConnection,
  connectionErrorMessage,
  readConnection,
  type Connection,
  type ConnectionAction,
} from "./api"

type RequestKind = ConnectionAction["type"] | "read"
export type ConnectionState = {
  connection: Connection | null
  pending: RequestKind | null
  message: string | null
  paused: boolean
}
type Dependencies = {
  session: Pick<SessionController, "getSnapshot" | "subscribe">
  visible: () => boolean
  online: () => boolean
  available: () => boolean
  read: typeof readConnection
  change: typeof changeConnection
}

/** In-memory view only. Resuming the page reads an operation; it never starts OAuth. */
export class ConnectionController {
  private state: ConnectionState = {
    connection: null,
    pending: null,
    message: null,
    paused: true,
  }
  private listeners = new Set<() => void>()
  private session?: Session
  private generation = 0
  private request?: AbortController
  private poll?: ReturnType<typeof setTimeout>
  private expiry?: ReturnType<typeof setTimeout>
  private unsubscribe?: () => void

  constructor(private dependencies: Dependencies) {}
  getSnapshot = () => this.state
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }
  private set(state: ConnectionState) {
    this.state = state
    for (const listener of this.listeners) listener()
  }
  private cancelRequest() {
    this.generation++
    this.request?.abort()
    this.request = undefined
    clearTimeout(this.poll)
    clearTimeout(this.expiry)
  }
  connect() {
    this.unsubscribe ??= this.dependencies.session.subscribe(this.synchronize)
    this.synchronize()
  }
  synchronize = () => {
    const auth = this.dependencies.session.getSnapshot()
    const session =
      auth.status === "active" &&
      this.dependencies.visible() &&
      this.dependencies.online() &&
      this.dependencies.available()
        ? auth.session
        : undefined
    if (session === this.session) return
    this.cancelRequest()
    this.session = session
    this.set({
      connection: null,
      pending: null,
      message: null,
      paused: !session,
    })
    if (session) void this.run("read")
  }
  refresh = async () => {
    if (this.state.pending && this.state.pending !== "read") return
    await this.run("read")
  }
  login = async () => {
    const status = this.state.connection?.status
    if (status !== "disconnected" && status !== "error") return
    if (this.state.pending && this.state.pending !== "read") return
    await this.run({ type: "login" })
  }
  cancelLogin = async () => {
    const challenge = this.state.connection?.challenge
    if (!challenge || (this.state.pending && this.state.pending !== "read"))
      return
    await this.run({ type: "cancel", operationId: challenge.operationId })
  }
  logout = async () => {
    if (!canDisconnectConnection(this.state.connection)) return
    if (this.state.pending && this.state.pending !== "read") return
    await this.run({ type: "logout" })
  }
  private expire() {
    this.cancelRequest()
    this.set({
      connection: { provider: "codex", status: "error", errorCode: "expired" },
      pending: null,
      message: null,
      paused: false,
    })
  }
  private observe(connection: Connection) {
    const remaining = connection.challenge
      ? Date.parse(connection.challenge.expiresAt) - Date.now()
      : null
    if (remaining !== null && remaining <= 0) {
      this.expire()
      return
    }
    this.set({ connection, pending: null, message: null, paused: false })
    if (connection.status === "connecting" && remaining !== null) {
      this.poll = setTimeout(() => {
        void this.run("read")
      }, 2_000)
      this.expiry = setTimeout(
        () => this.expire(),
        Math.min(remaining, 2_147_483_647)
      )
    }
  }
  private async run(action: "read" | ConnectionAction) {
    const auth = this.dependencies.session.getSnapshot()
    if (
      auth.status !== "active" ||
      auth.session !== this.session ||
      !this.dependencies.visible() ||
      !this.dependencies.online() ||
      !this.dependencies.available()
    ) {
      this.synchronize()
      return
    }
    this.cancelRequest()
    const generation = this.generation
    const request = new AbortController()
    this.request = request
    const isRead = action === "read"
    const connection = isRead ? this.state.connection : null
    this.set({
      connection,
      pending: isRead ? "read" : action.type,
      message: null,
      paused: false,
    })
    // A stalled status read must not keep an expired device code visible.
    if (connection?.challenge) {
      const remaining = Date.parse(connection.challenge.expiresAt) - Date.now()
      if (remaining <= 0) {
        this.expire()
        return
      }
      this.expiry = setTimeout(
        () => this.expire(),
        Math.min(remaining, 2_147_483_647)
      )
    }
    try {
      const result = isRead
        ? await this.dependencies.read(request.signal)
        : await this.dependencies.change(
            action,
            auth.session.csrfToken,
            request.signal
          )
      if (generation !== this.generation) return
      clearTimeout(this.expiry)
      this.observe(result)
    } catch (error) {
      if (generation !== this.generation) return
      clearTimeout(this.expiry)
      this.set({
        connection: null,
        pending: null,
        message: connectionErrorMessage(error),
        paused: false,
      })
    }
  }
  dispose() {
    this.unsubscribe?.()
    this.unsubscribe = undefined
    this.cancelRequest()
    this.session = undefined
    this.set({ connection: null, pending: null, message: null, paused: true })
  }
}
