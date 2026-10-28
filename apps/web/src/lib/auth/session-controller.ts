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
  | { status: "active"; session: Session }

type SessionDependencies = {
  read: typeof readSession
  revoke: typeof revokeSession
  clearPrivateData: () => void
  announceChange: () => void
  online: () => boolean
}

export class SessionController {
  private state: SessionState = { status: "checking" }
  private listeners = new Set<() => void>()
  private generation = 0
  private request?: AbortController
  private expiry?: ReturnType<typeof setTimeout>
  private lastUserId?: string
  private pendingLogout?: string

  constructor(private dependencies: SessionDependencies) {}

  getSnapshot = () => this.state
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

  private cancel() {
    this.generation++
    this.request?.abort()
    clearTimeout(this.expiry)
  }

  invalidate(status: "anonymous" | "offline" | "checking" = "anonymous") {
    this.cancel()
    this.dependencies.clearPrivateData()
    this.setState({ status: this.pendingLogout ? "logout-failed" : status })
  }

  authorizationFailed() {
    this.invalidate()
    this.dependencies.announceChange()
  }

  async invalidateRemote(): Promise<void> {
    this.invalidate("checking")
    await this.refresh("remote")
  }

  // A received invalidation must never echo, including its verification result.
  async refresh(source: "local" | "remote" = "local"): Promise<void> {
    if (this.pendingLogout) return
    if (!this.dependencies.online()) {
      this.invalidate("offline")
      return
    }
    this.cancel()
    const generation = this.generation
    this.request = new AbortController()
    this.setState({ status: "checking" })
    try {
      const session = await this.dependencies.read(this.request.signal)
      if (generation !== this.generation) return
      const remaining = Date.parse(session.expiresAt) - Date.now()
      if (!Number.isFinite(remaining) || remaining <= 0) {
        this.invalidate()
        return
      }
      if (session.user.id !== this.lastUserId) {
        this.dependencies.clearPrivateData()
        this.lastUserId = session.user.id
        if (source === "local") this.dependencies.announceChange()
      }
      this.setState({ status: "active", session })
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
      this.dependencies.clearPrivateData()
      this.setState({ status: "unavailable" })
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
    this.dependencies.announceChange()
  }

  dispose() {
    this.cancel()
  }
}
