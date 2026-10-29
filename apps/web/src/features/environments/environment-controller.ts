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
export function canStopEnvironment(environment: Environment | null): boolean {
  const status = environment?.status
  return (
    status === "starting" ||
    status === "running" ||
    status === "error" ||
    status === "unavailable"
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
    if (this.state.pending && this.state.pending !== "read") return
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
    } catch (error) {
      if (generation !== this.generation) return
      // Even a failed write may have reached the server. Never retain a Running
      // badge until a fresh response confirms the state.
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
    this.session = undefined
    this.setState({
      environment: null,
      pending: null,
      message: null,
      paused: true,
    })
  }
}
