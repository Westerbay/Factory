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
function canStartEnvironment(environment: Environment | null): boolean {
  return (
    environment?.status === "not_created" || environment?.status === "stopped"
  )
}
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
  hasRecoverableRuns: (signal: AbortSignal) => Promise<boolean>
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
  private startupSession?: { userId: string; csrfToken: string }
  private startupConsumed = false
  private retryInitialization = false
  private startupMessage: string | null = null

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
    if (auth.status === "anonymous") this.startupSession = undefined
    const session =
      auth.status === "active" && this.dependencies.online()
        ? auth.session
        : undefined
    const sameSession =
      session !== undefined &&
      session.user.id === this.session?.user.id &&
      session.csrfToken === this.session.csrfToken &&
      session.user.role === this.session.user.role
    const paused = !session || !this.dependencies.visible()
    if (
      (sameSession || (!session && !this.session)) &&
      paused === this.state.paused
    ) {
      this.session = session
      return
    }
    // Pausing observation must not make a running workspace disappear. A write
    // with an unknown outcome still needs a fresh read before it can be trusted.
    const retainObservation =
      sameSession && (!this.state.pending || this.state.pending === "read")
    this.cancel()
    clearInterval(this.poll)
    if (
      session &&
      (session.user.id !== this.startupSession?.userId ||
        session.csrfToken !== this.startupSession?.csrfToken)
    ) {
      this.startupSession = {
        userId: session.user.id,
        csrfToken: session.csrfToken,
      }
      this.startupConsumed = false
      this.retryInitialization = false
      this.startupMessage = null
    }
    this.session = session
    this.setState({
      environment: retainObservation ? this.state.environment : null,
      pending: null,
      message: retainObservation
        ? this.state.message
        : session
          ? this.startupMessage
          : null,
      paused,
    })
    if (paused) return
    void this.run("read")
    this.poll = setInterval(() => {
      if (!this.state.pending) void this.run("read")
    }, 5_000)
  }

  private cancel() {
    // Initialization cannot restart an existing environment. Retry only when a
    // later read still proves absence, never a subsequently stopped environment.
    if (this.state.pending === "initialize") this.retryInitialization = true
    this.generation++
    this.request?.abort()
    this.request = undefined
  }

  retry = async () => {
    if (this.state.pending && this.state.pending !== "read") return
    const reading = this.run("read", true)
    const generation = this.generation
    await reading
    if (
      generation !== this.generation ||
      this.state.message ||
      this.state.paused
    )
      return
    await this.start()
  }

  start = async () => {
    if (!canStartEnvironment(this.state.environment)) return
    if (this.state.pending && this.state.pending !== "read") return
    this.startupConsumed = true
    await this.run(
      this.state.environment?.status === "not_created" ? "initialize" : "start",
      true
    )
  }

  stop = async () => {
    if (!canStopEnvironment(this.state.environment)) return
    if (this.state.pending && this.state.pending !== "read") return
    this.startupConsumed = true
    this.retryInitialization = false
    await this.run("stop", true)
  }

  private async checkRecoveryBeforeStart(
    signal: AbortSignal,
    generation: number
  ) {
    let message: string | null = null
    try {
      if (await this.dependencies.hasRecoverableRuns(signal))
        message =
          "A previous task needs recovery before your workspace can start."
    } catch {
      message =
        "Recovery status could not be confirmed. Try again before starting your workspace."
    }
    if (generation !== this.generation) return false
    if (!message) return true
    this.startupMessage = message
    this.setState({ ...this.state, pending: null, message })
    return false
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
    if (clearMessage) this.startupMessage = null
    this.cancel()
    if (kind === "initialize") this.retryInitialization = false
    const generation = this.generation
    const request = new AbortController()
    this.request = request
    this.setState({
      ...this.state,
      pending: kind,
      message: clearMessage ? null : this.state.message,
    })
    // A stopped observation is required for recovery. Do not restart until the
    // existing recovery API confirms there are no outstanding runs to resolve.
    if (
      kind === "start" &&
      !(await this.checkRecoveryBeforeStart(request.signal, generation))
    )
      return
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
      if (environment.status === "running") this.startupMessage = null
      this.setState({
        ...this.state,
        environment,
        pending: null,
        message: kind === "read" ? this.startupMessage : null,
      })
      if (kind === "read") {
        const firstEntry = !this.startupConsumed
        // Also consume entry when already running/stopping: polling must not
        // reverse a stop requested from another tab or a later runtime failure.
        this.startupConsumed = true
        if (environment.status !== "not_created")
          this.retryInitialization = false
        if (
          environment.status === "not_created" &&
          (firstEntry || this.retryInitialization)
        )
          await this.run("initialize")
        else if (firstEntry && environment.status === "stopped")
          await this.run("start")
      }
    } catch (error) {
      if (generation !== this.generation) return
      const message = environmentErrorMessage(error)
      if (kind === "initialize" || kind === "start")
        this.startupMessage = message
      // Even a failed write may have reached the server. Never retain a Running
      // badge until a fresh response confirms the state.
      this.setState({
        environment: null,
        pending: null,
        paused: false,
        message,
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
