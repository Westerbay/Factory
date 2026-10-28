import type { Session } from "../../lib/auth/session-api"
import type { SessionController } from "../../lib/auth/session-controller"
import {
  changeEnvironment,
  environmentErrorMessage,
  readEnvironment,
  type Environment,
  type EnvironmentAction,
} from "./api"

type RequestKind = EnvironmentAction | "read"
const HEARTBEAT_INTERVAL_MS = 60_000
const LEASE_MARGIN_MS = 10_000

function renewableLeaseExpiresAt(
  environment: Environment | null
): number | null {
  if (environment?.status !== "starting" && environment?.status !== "running")
    return null
  const expiresAt = Date.parse(environment.idleExpiresAt ?? "")
  return Number.isFinite(expiresAt) && expiresAt > Date.now() ? expiresAt : null
}

export function canStopEnvironment(environment: Environment | null): boolean {
  const status = environment?.status
  return (
    status === "starting" ||
    status === "running" ||
    status === "error" ||
    (status === "unavailable" && environment?.idleExpiresAt != null)
  )
}

export type EnvironmentState = {
  environment: Environment | null
  pending: RequestKind | null
  message: string | null
  paused: boolean
}
type Dependencies = {
  session: Pick<SessionController, "getSnapshot" | "subscribe">
  visible: () => boolean
  online: () => boolean
  read: typeof readEnvironment
  change: typeof changeEnvironment
}

export class EnvironmentController {
  private state: EnvironmentState = {
    environment: null,
    pending: null,
    message: null,
    paused: true,
  }
  private listeners = new Set<() => void>()
  private session?: Session
  private generation = 0
  private request?: AbortController
  private poll?: ReturnType<typeof setInterval>
  private heartbeat?: ReturnType<typeof setTimeout>
  private heartbeatDue?: number
  private renewOnRead = true
  private unsubscribe?: () => void

  constructor(private dependencies: Dependencies) {}

  getSnapshot = () => this.state
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  private setState(state: EnvironmentState) {
    this.state = state
    for (const listener of this.listeners) listener()
  }

  connect() {
    this.unsubscribe = this.dependencies.session.subscribe(this.synchronize)
    this.synchronize()
  }

  // Subscribe directly to verified session changes, including logout's initial
  // invalidation, so cancellation does not wait for a React effect cleanup.
  synchronize = () => {
    const auth = this.dependencies.session.getSnapshot()
    const session =
      auth.status === "active" &&
      this.dependencies.visible() &&
      this.dependencies.online()
        ? auth.session
        : undefined
    if (session === this.session) return
    this.cancel()
    clearInterval(this.poll)
    this.clearHeartbeat()
    this.renewOnRead = true
    this.session = session
    this.setState({
      environment: null,
      pending: null,
      message: null,
      paused: !session,
    })
    if (!session) return
    void this.run("read")
    this.poll = setInterval(() => {
      if (!this.state.pending) void this.run("read")
    }, 5_000)
  }

  private clearHeartbeat() {
    clearTimeout(this.heartbeat)
    this.heartbeat = undefined
    this.heartbeatDue = undefined
  }

  private scheduleHeartbeat(kind: RequestKind) {
    const expiresAt = renewableLeaseExpiresAt(this.state.environment)
    if (expiresAt === null || kind === "stop") {
      this.clearHeartbeat()
      return
    }
    if (kind === "read" && this.renewOnRead) {
      this.renewOnRead = false
      void this.run("heartbeat")
      return
    }
    this.renewOnRead = false
    const now = Date.now()
    const remaining = expiresAt - now
    // Leave room for transport latency even with a one-minute lease. The floor
    // prevents a malformed or nearly expired response from creating a busy loop.
    const delay = Math.max(
      1_000,
      Math.min(
        HEARTBEAT_INTERVAL_MS,
        remaining - Math.min(LEASE_MARGIN_MS, remaining / 2)
      )
    )
    const due = now + delay
    // Reads confirm state every five seconds; they must never postpone renewal.
    if (this.heartbeatDue !== undefined && this.heartbeatDue <= due) return
    this.clearHeartbeat()
    this.heartbeatDue = due
    this.heartbeat = setTimeout(() => {
      this.clearHeartbeat()
      void this.run("heartbeat")
    }, delay)
  }

  private cancel() {
    this.generation++
    this.request?.abort()
    this.request = undefined
  }

  refresh = async () => {
    if (this.state.pending && this.state.pending !== "read") return
    await this.run("read", true)
  }

  start = async () => {
    const status = this.state.environment?.status
    if (status !== "not_created" && status !== "stopped") return
    await this.act("start")
  }

  stop = async () => {
    if (!canStopEnvironment(this.state.environment)) return
    await this.act("stop")
  }

  private async act(action: "start" | "stop") {
    if (
      this.state.pending &&
      this.state.pending !== "read" &&
      this.state.pending !== "heartbeat"
    )
      return
    await this.run(action, true)
  }

  private async run(kind: RequestKind, clearMessage = false) {
    const auth = this.dependencies.session.getSnapshot()
    if (
      auth.status !== "active" ||
      auth.session !== this.session ||
      !this.dependencies.visible() ||
      !this.dependencies.online()
    ) {
      this.synchronize()
      return
    }
    const session = this.session
    if (!session) return
    if (
      kind === "heartbeat" &&
      renewableLeaseExpiresAt(this.state.environment) === null
    ) {
      this.clearHeartbeat()
      return
    }
    if (kind !== "read") this.clearHeartbeat()
    this.cancel()
    const generation = this.generation
    const request = new AbortController()
    this.request = request
    this.setState({
      ...this.state,
      pending: kind,
      message: clearMessage ? null : this.state.message,
    })
    try {
      const environment =
        kind === "read"
          ? await this.dependencies.read(request.signal)
          : await this.dependencies.change(
              kind,
              session.csrfToken,
              request.signal
            )
      if (generation !== this.generation) return
      this.setState({
        ...this.state,
        environment,
        pending: null,
        message: null,
      })
      if (generation === this.generation) this.scheduleHeartbeat(kind)
    } catch (error) {
      if (generation !== this.generation) return
      this.clearHeartbeat()
      this.renewOnRead = true
      // Even a failed write may have reached the server. Never retain a Running
      // badge or renew its lease until a fresh response confirms the state.
      this.setState({
        environment: null,
        pending: null,
        paused: false,
        message: environmentErrorMessage(error),
      })
    }
  }

  dispose() {
    this.unsubscribe?.()
    this.unsubscribe = undefined
    this.cancel()
    clearInterval(this.poll)
    this.clearHeartbeat()
    this.session = undefined
    this.setState({
      environment: null,
      pending: null,
      message: null,
      paused: true,
    })
  }
}
