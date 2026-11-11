import { ApiFailure } from "../api/client"
import { readSession, revokeSession, type Session } from "./session-api"

export type SessionState =
  | {
      status:
        | "checking"
        | "anonymous"
        | "unavailable"
        | "offline"
        | "logout-failed"
    }
  | {
      status: "active"
      session: Session
      verification?: "offline" | "unavailable"
    }

export type SessionChange = "invalidate" | "recheck"

type SessionDependencies = {
  read: typeof readSession
  revoke: typeof revokeSession
  clearPrivateData: () => void
  announceChange: (change: SessionChange) => void
  online: () => boolean
}

const verificationRetryDelays = [750, 2_000, 5_000]

export class SessionController {
  private state: SessionState = { status: "checking" }
  private listeners = new Set<() => void>()
  private generation = 0
  private request?: AbortController
  private pendingRefresh?: Promise<void>
  private expiry?: ReturnType<typeof setTimeout>
  private retry?: ReturnType<typeof setTimeout>
  private lastUserId?: string
  private pendingLogout?: string
  private scope = 0
  private scopeIdentity?: {
    userId: string
    csrfToken: string
    role: Session["user"]["role"]
  }

  constructor(private dependencies: SessionDependencies) {}

  getSnapshot = () => this.state
  // Presentation identity only; a different security identity starts a new tree.
  getScope = () => (this.scopeIdentity ? this.scope : undefined)
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  private setState(state: SessionState) {
    this.state = state
    for (const listener of this.listeners) listener()
  }

  private cancel(clearExpiry = true) {
    this.generation++
    clearTimeout(this.retry)
    this.request?.abort()
    this.pendingRefresh = undefined
    if (clearExpiry) clearTimeout(this.expiry)
  }

  invalidate(
    status: "anonymous" | "offline" | "unavailable" | "checking" = "anonymous"
  ) {
    this.cancel()
    this.scopeIdentity = undefined
    this.dependencies.clearPrivateData()
    this.setState({ status: this.pendingLogout ? "logout-failed" : status })
  }

  authorizationFailed() {
    this.invalidate()
    this.dependencies.announceChange("invalidate")
  }

  async invalidateRemote(): Promise<void> {
    this.invalidate("checking")
    await this.refresh("remote")
  }

  private verificationFailed(status: "offline" | "unavailable") {
    if (
      this.state.status === "active" &&
      Date.parse(this.state.session.expiresAt) > Date.now()
    ) {
      if (this.state.verification !== status)
        this.setState({ ...this.state, verification: status })
    } else this.invalidate(status)
  }

  // Received signals must never echo, including their verification results.
  refresh(source: "local" | "remote" = "local"): Promise<void> {
    return this.recheck(source, 0)
  }

  private async recheck(
    source: "local" | "remote",
    retryIndex: number
  ): Promise<void> {
    if (this.pendingLogout) return
    if (
      this.state.status === "active" &&
      Date.parse(this.state.session.expiresAt) <= Date.now()
    )
      this.invalidate()
    if (!this.dependencies.online()) {
      this.cancel(this.state.status !== "active")
      this.verificationFailed("offline")
      return
    }
    if (this.pendingRefresh) return this.pendingRefresh
    const background = this.state.status === "active"
    // A background recheck leaves the verified session and its fixed expiry intact.
    this.cancel(!background)
    const generation = this.generation
    this.request = new AbortController()
    if (!background && retryIndex === 0 && this.state.status !== "checking")
      this.setState({ status: "checking" })
    const pending = this.verify(
      source,
      generation,
      this.request.signal,
      retryIndex
    )
    this.pendingRefresh = pending
    try {
      await pending
    } finally {
      if (generation === this.generation) this.pendingRefresh = undefined
    }
  }

  private async verify(
    source: "local" | "remote",
    generation: number,
    signal: AbortSignal,
    retryIndex: number
  ): Promise<void> {
    try {
      const session = await this.dependencies.read(signal)
      if (generation !== this.generation) return
      const remaining = Date.parse(session.expiresAt) - Date.now()
      if (!Number.isFinite(remaining) || remaining <= 0) {
        this.invalidate()
        return
      }
      const sameScope =
        this.scopeIdentity?.userId === session.user.id &&
        this.scopeIdentity.csrfToken === session.csrfToken &&
        this.scopeIdentity.role === session.user.role
      if (
        session.user.id !== this.lastUserId ||
        (this.scopeIdentity && !sameScope)
      )
        this.dependencies.clearPrivateData()
      const previousUser = this.lastUserId
      this.lastUserId = session.user.id
      if (source === "local" && session.user.id !== previousUser) {
        this.dependencies.announceChange(
          previousUser ? "invalidate" : "recheck"
        )
      }
      if (!sameScope) {
        this.scope++
        this.scopeIdentity = {
          userId: session.user.id,
          csrfToken: session.csrfToken,
          role: session.user.role,
        }
      }
      clearTimeout(this.expiry)
      const previous =
        this.state.status === "active" ? this.state.session : undefined
      const unchanged =
        sameScope &&
        previous?.expiresAt === session.expiresAt &&
        previous.user.handle === session.user.handle &&
        previous.user.displayName === session.user.displayName
      // Consumers use the session reference to synchronize their transports.
      // A successful unchanged check must not restart those resources.
      if (!unchanged) this.setState({ status: "active", session })
      else if (this.state.status === "active" && this.state.verification)
        this.setState({ status: "active", session: previous })
      this.expiry = setTimeout(
        () => this.invalidate(),
        Math.min(remaining, 2_147_483_647)
      )
    } catch (error) {
      if (generation !== this.generation) return
      if (error instanceof ApiFailure && [401, 403].includes(error.status)) {
        if (source === "local") this.authorizationFailed()
        else this.invalidate()
        return
      }
      const status = this.dependencies.online() ? "unavailable" : "offline"
      this.verificationFailed(status)
      const delay = verificationRetryDelays[retryIndex]
      if (status === "unavailable" && delay !== undefined) {
        this.retry = setTimeout(() => {
          void this.recheck(source, retryIndex + 1)
        }, delay)
      }
    }
  }

  async logout(): Promise<void> {
    const token =
      this.pendingLogout ??
      (this.state.status === "active"
        ? this.state.session.csrfToken
        : undefined)
    if (!token) return
    this.pendingLogout = token
    this.invalidate("checking")
    this.setState({ status: "checking" })
    const generation = this.generation
    try {
      await this.dependencies.revoke(token)
    } catch (error) {
      if (!(error instanceof ApiFailure && error.status === 401)) {
        if (generation === this.generation)
          this.setState({ status: "logout-failed" })
        return
      }
    }
    this.pendingLogout = undefined
    this.invalidate()
    this.dependencies.announceChange("invalidate")
  }

  dispose() {
    this.cancel()
  }
}
