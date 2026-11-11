import { afterEach, describe, expect, it, vi } from "vitest"

import { ApiFailure } from "../../src/lib/api/client"
import { SessionController } from "../../src/lib/auth/session-controller"
import type { Session } from "../../src/lib/auth/session-api"

const session = (id = "d3d81ae5-5ee6-4a75-bb54-126bfe991643"): Session => ({
  user: { id, handle: "member", displayName: "Test Member", role: "member" },
  csrfToken: "synthetic-csrf",
  expiresAt: new Date(Date.now() + 60_000).toISOString(),
})
const controllers: SessionController[] = []
afterEach(() => {
  for (const controller of controllers.splice(0)) controller.dispose()
  vi.useRealTimers()
})

function setup(
  read = vi
    .fn<(signal: AbortSignal) => Promise<Session>>()
    .mockResolvedValue(session())
) {
  const dependencies = {
    read,
    revoke: vi
      .fn<(token: string) => Promise<void>>()
      .mockResolvedValue(undefined),
    clearPrivateData: vi.fn(),
    announceChange: vi.fn(),
    online: vi.fn().mockReturnValue(true),
  }
  const controller = new SessionController(dependencies)
  controllers.push(controller)
  return { controller, dependencies }
}

describe("private session state", () => {
  it("shares the initial verification requested by multiple consumers", async () => {
    let complete!: (value: Session) => void
    const read = vi.fn<() => Promise<Session>>().mockImplementation(
      () =>
        new Promise((resolve) => {
          complete = resolve
        })
    )
    const { controller } = setup(read)
    const first = controller.refresh()
    const second = controller.refresh()
    expect(read).toHaveBeenCalledOnce()
    expect(controller.getSnapshot()).toEqual({ status: "checking" })
    complete(session())
    await Promise.all([first, second])
    expect(controller.getSnapshot().status).toBe("active")
  })

  it("coalesces routine checks without changing a verified snapshot or restarting consumers", async () => {
    const { controller, dependencies } = setup()
    expect(controller.getScope()).toBeUndefined()
    await controller.refresh()
    const previous = controller.getSnapshot()
    if (previous.status !== "active") throw new Error("Expected active session")
    const scope = controller.getScope()
    dependencies.clearPrivateData.mockClear()
    const listener = vi.fn()
    controller.subscribe(listener)
    let complete!: (value: Session) => void
    dependencies.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          complete = resolve
        })
    )
    const recheck = controller.refresh()
    const repeated = controller.refresh()
    expect(controller.getSnapshot()).toBe(previous)
    expect(dependencies.read).toHaveBeenCalledTimes(2)
    complete({ ...previous.session, user: { ...previous.session.user } })
    await Promise.all([recheck, repeated])
    expect(controller.getSnapshot()).toBe(previous)
    expect(controller.getScope()).toBe(scope)
    expect(dependencies.clearPrivateData).not.toHaveBeenCalled()
    expect(listener).not.toHaveBeenCalled()
  })

  it.each(["user", "token", "role"] as const)(
    "replaces the presentation scope and private cache when the verified %s changes",
    async (change) => {
      const { controller, dependencies } = setup()
      await controller.refresh()
      const scope = controller.getScope()
      dependencies.clearPrivateData.mockClear()
      const next = session()
      if (change === "user")
        next.user.id = "a3d81ae5-5ee6-4a75-bb54-126bfe991643"
      if (change === "token") next.csrfToken = "new-synthetic-csrf"
      if (change === "role") next.user.role = "admin"
      dependencies.read.mockResolvedValueOnce(next)
      await controller.refresh()
      expect(controller.getScope()).not.toBe(scope)
      expect(dependencies.clearPrivateData).toHaveBeenCalledOnce()
    }
  )

  it.each(["handle", "displayName", "expiresAt"] as const)(
    "updates verified %s without replacing the private scope",
    async (field) => {
      const { controller, dependencies } = setup()
      await controller.refresh()
      const previous = controller.getSnapshot()
      if (previous.status !== "active")
        throw new Error("Expected active session")
      const scope = controller.getScope()
      dependencies.clearPrivateData.mockClear()
      const next = { ...previous.session, user: { ...previous.session.user } }
      if (field === "expiresAt")
        next.expiresAt = new Date(
          Date.parse(next.expiresAt) - 1000
        ).toISOString()
      else next.user[field] = "Updated member"
      dependencies.read.mockResolvedValueOnce(next)
      await controller.refresh()
      expect(controller.getSnapshot()).toEqual({
        status: "active",
        session: next,
      })
      expect(controller.getScope()).toBe(scope)
      expect(dependencies.clearPrivateData).not.toHaveBeenCalled()
    }
  )

  it("drops the active scope immediately on remote invalidation", async () => {
    const { controller, dependencies } = setup()
    await controller.refresh()
    const previousScope = controller.getScope()
    let completeOld!: (value: Session) => void
    let completeNew!: (value: Session) => void
    dependencies.read
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            completeOld = resolve
          })
      )
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            completeNew = resolve
          })
      )
    const pending = controller.refresh()
    expect(controller.getSnapshot().status).toBe("active")
    const remote = controller.invalidateRemote()
    expect(controller.getSnapshot()).toEqual({ status: "checking" })
    expect(controller.getScope()).toBeUndefined()
    completeOld(session())
    await pending
    expect(controller.getScope()).toBeUndefined()
    const repeated = controller.refresh()
    expect(dependencies.read).toHaveBeenCalledTimes(3)
    completeNew(session())
    await Promise.all([remote, repeated])
    expect(controller.getScope()).not.toBe(previousScope)
  })

  it("keeps the original expiry deadline while repeated focus checks are pending", async () => {
    vi.useFakeTimers()
    const { controller, dependencies } = setup()
    await controller.refresh()
    let complete!: (value: Session) => void
    dependencies.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          complete = resolve
        })
    )
    await vi.advanceTimersByTimeAsync(30_000)
    const pending = controller.refresh()
    expect(controller.getSnapshot().status).toBe("active")
    await vi.advanceTimersByTimeAsync(20_000)
    const later = controller.refresh()
    expect(controller.getSnapshot().status).toBe("active")
    expect(dependencies.read).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(10_001)
    expect(controller.getSnapshot()).toEqual({ status: "anonymous" })
    expect(controller.getScope()).toBeUndefined()
    expect(dependencies.read.mock.calls[1]?.[0].aborted).toBe(true)
    complete(session())
    await Promise.all([pending, later])
    expect(controller.getSnapshot()).toEqual({ status: "anonymous" })
  })

  it("never restores an old login response after invalidation or a newer request", async () => {
    let resolveOld!: (value: Session) => void
    const read = vi
      .fn<() => Promise<Session>>()
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveOld = resolve
          })
      )
      .mockResolvedValueOnce(session("a3d81ae5-5ee6-4a75-bb54-126bfe991643"))
    const { controller, dependencies } = setup(read)
    const old = controller.refresh()
    controller.invalidate("offline")
    await controller.refresh()
    resolveOld(session())
    await old
    expect(controller.getSnapshot()).toMatchObject({
      status: "active",
      session: { user: { id: "a3d81ae5-5ee6-4a75-bb54-126bfe991643" } },
    })
    expect(dependencies.clearPrivateData).toHaveBeenCalled()
  })

  it("keeps an existing view mounted when another tab opens the same session", async () => {
    const first = setup()
    const second = setup()
    await first.controller.refresh()
    const snapshot = first.controller.getSnapshot()
    let recheck: Promise<void> | undefined
    second.dependencies.announceChange.mockImplementation((change) => {
      if (change === "recheck") recheck = first.controller.refresh("remote")
      else recheck = first.controller.invalidateRemote()
      expect(first.controller.getSnapshot()).toBe(snapshot)
    })
    await second.controller.refresh()
    await recheck
    expect(second.dependencies.announceChange).toHaveBeenCalledWith("recheck")
    expect(first.controller.getSnapshot().status).toBe("active")
    expect(first.dependencies.announceChange).toHaveBeenCalledTimes(1)
  })

  it("recovers from a temporary verification failure without discarding the active session", async () => {
    vi.useFakeTimers()
    const { controller, dependencies } = setup()
    await controller.refresh()
    const snapshot = controller.getSnapshot()
    const scope = controller.getScope()
    dependencies.clearPrivateData.mockClear()
    dependencies.read.mockRejectedValueOnce(new ApiFailure(503))
    await controller.refresh()
    expect(controller.getSnapshot()).toEqual({
      ...snapshot,
      verification: "unavailable",
    })
    await vi.advanceTimersByTimeAsync(750)
    expect(controller.getSnapshot()).toEqual(snapshot)
    expect(controller.getScope()).toBe(scope)
    expect(dependencies.clearPrivateData).not.toHaveBeenCalled()
  })

  it("bounds temporary retries and stops when the server refuses access", async () => {
    vi.useFakeTimers()
    const { controller, dependencies } = setup()
    dependencies.read.mockRejectedValue(new ApiFailure(503))
    await controller.refresh()
    await vi.advanceTimersByTimeAsync(12_000)
    expect(dependencies.read).toHaveBeenCalledTimes(4)
    expect(controller.getSnapshot()).toEqual({ status: "unavailable" })
    dependencies.read.mockRejectedValueOnce(new ApiFailure(401))
    await controller.refresh()
    await vi.advanceTimersByTimeAsync(12_000)
    expect(dependencies.read).toHaveBeenCalledTimes(5)
    expect(controller.getSnapshot()).toEqual({ status: "anonymous" })
  })

  it("clears cached private data when the verified account changes", async () => {
    const { controller, dependencies } = setup()
    await controller.refresh()
    dependencies.clearPrivateData.mockClear()
    dependencies.read.mockResolvedValueOnce(
      session("a3d81ae5-5ee6-4a75-bb54-126bfe991643")
    )
    await controller.refresh()
    expect(dependencies.clearPrivateData).toHaveBeenCalledOnce()
    expect(dependencies.announceChange.mock.calls).toEqual([
      ["recheck"],
      ["invalidate"],
    ])
  })

  it.each([401, 403])(
    "propagates a local HTTP %s refusal without echoes or restoring an old response",
    async (status) => {
      const first = setup()
      const second = setup()
      await Promise.all([
        first.controller.refresh(),
        second.controller.refresh(),
      ])
      first.dependencies.announceChange.mockClear()
      second.dependencies.announceChange.mockClear()
      second.dependencies.clearPrivateData.mockClear()

      let resolveOld!: (value: Session) => void
      second.dependencies.read
        .mockImplementationOnce(
          () =>
            new Promise((resolve) => {
              resolveOld = resolve
            })
        )
        .mockRejectedValueOnce(new ApiFailure(status))
      const oldRequest = second.controller.refresh()
      const received: Promise<void>[] = []
      first.dependencies.announceChange.mockImplementation(() => {
        received.push(second.controller.invalidateRemote())
        expect(second.controller.getSnapshot()).toEqual({ status: "checking" })
        expect(second.dependencies.clearPrivateData).toHaveBeenCalled()
      })
      first.dependencies.read.mockRejectedValueOnce(new ApiFailure(status))
      await first.controller.refresh()
      await Promise.all(received)
      resolveOld(session())
      await oldRequest

      expect(first.controller.getSnapshot()).toEqual({ status: "anonymous" })
      expect(second.controller.getSnapshot()).toEqual({ status: "anonymous" })
      expect(first.dependencies.announceChange).toHaveBeenCalledOnce()
      expect(second.dependencies.announceChange).not.toHaveBeenCalled()
    }
  )

  it("broadcasts a protected API refusal and immediately hides another active tab", async () => {
    const first = setup()
    const second = setup()
    await Promise.all([first.controller.refresh(), second.controller.refresh()])
    first.dependencies.announceChange.mockClear()
    second.dependencies.announceChange.mockClear()
    second.dependencies.clearPrivateData.mockClear()
    second.dependencies.read.mockRejectedValueOnce(new ApiFailure(401))
    let received!: Promise<void>
    first.dependencies.announceChange.mockImplementation(() => {
      received = second.controller.invalidateRemote()
    })

    first.controller.authorizationFailed()
    expect(first.controller.getSnapshot()).toEqual({ status: "anonymous" })
    expect(second.controller.getSnapshot()).toEqual({ status: "checking" })
    expect(second.dependencies.clearPrivateData).toHaveBeenCalledOnce()
    await received
    expect(second.controller.getSnapshot()).toEqual({ status: "anonymous" })
    expect(first.dependencies.announceChange).toHaveBeenCalledOnce()
    expect(second.dependencies.announceChange).not.toHaveBeenCalled()
  })

  it("does not rebroadcast a changed identity discovered through a remote invalidation", async () => {
    const { controller, dependencies } = setup()
    await controller.refresh()
    dependencies.announceChange.mockClear()
    dependencies.read.mockResolvedValueOnce(
      session("a3d81ae5-5ee6-4a75-bb54-126bfe991643")
    )
    await controller.invalidateRemote()
    expect(controller.getSnapshot()).toMatchObject({
      status: "active",
      session: { user: { id: "a3d81ae5-5ee6-4a75-bb54-126bfe991643" } },
    })
    expect(dependencies.announceChange).not.toHaveBeenCalled()
  })

  it.each([401, 403])(
    "hides and clears the previous session on HTTP %s",
    async (status) => {
      const { controller, dependencies } = setup()
      await controller.refresh()
      const previous = controller.getSnapshot()
      dependencies.read.mockRejectedValueOnce(new ApiFailure(status))
      const refresh = controller.refresh()
      expect(controller.getSnapshot()).toBe(previous)
      await refresh
      expect(controller.getSnapshot()).toEqual({ status: "anonymous" })
      expect(dependencies.clearPrivateData).toHaveBeenCalledTimes(2)
      expect(controller.getScope()).toBeUndefined()
    }
  )

  it.each([new ApiFailure(503), new TypeError("Network unavailable")])(
    "keeps an unexpired verified session on a transient check failure without extending it",
    async (error) => {
      vi.useFakeTimers()
      const { controller, dependencies } = setup()
      await controller.refresh()
      const previous = controller.getSnapshot()
      if (previous.status !== "active")
        throw new Error("Expected active session")
      const scope = controller.getScope()
      dependencies.clearPrivateData.mockClear()
      await vi.advanceTimersByTimeAsync(30_000)
      dependencies.read.mockRejectedValueOnce(error)
      await controller.refresh()
      const failed = controller.getSnapshot()
      expect(failed).toEqual({
        ...previous,
        verification: "unavailable",
      })
      if (failed.status !== "active") throw new Error("Expected active session")
      expect(failed.session).toBe(previous.session)
      expect(controller.getScope()).toBe(scope)
      expect(dependencies.clearPrivateData).not.toHaveBeenCalled()
      await vi.advanceTimersByTimeAsync(30_001)
      expect(controller.getSnapshot()).toEqual({ status: "anonymous" })
      expect(controller.getScope()).toBeUndefined()
      expect(dependencies.read).toHaveBeenCalledTimes(3)
      expect(dependencies.clearPrivateData).toHaveBeenCalledOnce()
    }
  )

  it("clears a failed verification notice after recovery without replacing the session", async () => {
    const { controller, dependencies } = setup()
    await controller.refresh()
    const previous = controller.getSnapshot()
    if (previous.status !== "active") throw new Error("Expected active session")
    dependencies.read.mockRejectedValueOnce(new ApiFailure(503))
    await controller.refresh()
    dependencies.read.mockResolvedValueOnce({
      ...previous.session,
      user: { ...previous.session.user },
    })
    await controller.refresh()
    const recovered = controller.getSnapshot()
    expect(recovered).toEqual(previous)
    if (recovered.status !== "active")
      throw new Error("Expected active session")
    expect(recovered.session).toBe(previous.session)
  })

  it("retains an unexpired offline session, aborts its check, and expires without retrying", async () => {
    vi.useFakeTimers()
    const { controller, dependencies } = setup()
    await controller.refresh()
    const previous = controller.getSnapshot()
    let complete!: (value: Session) => void
    dependencies.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          complete = resolve
        })
    )
    const checking = controller.refresh()
    await vi.advanceTimersByTimeAsync(30_000)
    dependencies.online.mockReturnValue(false)
    await controller.refresh()
    expect(controller.getSnapshot()).toEqual({
      ...previous,
      verification: "offline",
    })
    expect(dependencies.read.mock.calls[1]?.[0].aborted).toBe(true)
    complete(session())
    await checking
    expect(controller.getSnapshot()).toEqual({
      ...previous,
      verification: "offline",
    })
    await vi.advanceTimersByTimeAsync(30_001)
    expect(controller.getSnapshot()).toEqual({ status: "anonymous" })
    expect(dependencies.read).toHaveBeenCalledTimes(2)
  })

  it("does not grant an initial session after an unavailable or offline check", async () => {
    const { controller, dependencies } = setup()
    dependencies.read.mockRejectedValueOnce(new ApiFailure(503))
    await controller.refresh()
    expect(controller.getSnapshot()).toEqual({ status: "unavailable" })
    dependencies.online.mockReturnValue(false)
    await controller.refresh()
    expect(controller.getSnapshot()).toEqual({ status: "offline" })
    expect(controller.getScope()).toBeUndefined()
    expect(dependencies.read).toHaveBeenCalledOnce()
  })

  it("withdraws expired and offline sessions without exposing cached data", async () => {
    vi.useFakeTimers()
    const { controller, dependencies } = setup()
    await controller.refresh()
    await vi.advanceTimersByTimeAsync(60_001)
    expect(controller.getSnapshot()).toEqual({ status: "anonymous" })
    dependencies.online.mockReturnValue(false)
    await controller.refresh()
    expect(controller.getSnapshot()).toEqual({ status: "offline" })
    expect(dependencies.read).toHaveBeenCalledOnce()
  })

  it("hides immediately on logout, sends its CSRF value and announces confirmed revocation", async () => {
    const { controller, dependencies } = setup()
    await controller.refresh()
    dependencies.announceChange.mockClear()
    const logout = controller.logout()
    expect(controller.getSnapshot()).toEqual({ status: "checking" })
    expect(controller.getScope()).toBeUndefined()
    await logout
    expect(dependencies.revoke).toHaveBeenCalledWith("synthetic-csrf")
    expect(controller.getSnapshot()).toEqual({ status: "anonymous" })
    expect(dependencies.announceChange).toHaveBeenCalledOnce()
  })

  it("keeps failed logout hidden and retryable instead of re-admitting on focus", async () => {
    const { controller, dependencies } = setup()
    await controller.refresh()
    dependencies.revoke.mockRejectedValueOnce(new ApiFailure(503))
    await controller.logout()
    await controller.refresh()
    expect(controller.getSnapshot()).toEqual({ status: "logout-failed" })
    expect(dependencies.read).toHaveBeenCalledOnce()
    await controller.logout()
    expect(controller.getSnapshot()).toEqual({ status: "anonymous" })
    expect(dependencies.revoke).toHaveBeenCalledTimes(2)
  })
})
