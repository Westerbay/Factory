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
  read = vi.fn<() => Promise<Session>>().mockResolvedValue(session())
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

  it("clears cached private data when the verified account changes", async () => {
    const { controller, dependencies } = setup()
    await controller.refresh()
    dependencies.clearPrivateData.mockClear()
    dependencies.read.mockResolvedValueOnce(
      session("a3d81ae5-5ee6-4a75-bb54-126bfe991643")
    )
    await controller.refresh()
    expect(dependencies.clearPrivateData).toHaveBeenCalledOnce()
    expect(dependencies.announceChange).toHaveBeenCalledTimes(2)
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

  it.each([401, 403, 503])(
    "hides and clears the previous session on HTTP %s",
    async (status) => {
      const { controller, dependencies } = setup()
      await controller.refresh()
      dependencies.read.mockRejectedValueOnce(new ApiFailure(status))
      const refresh = controller.refresh()
      expect(controller.getSnapshot()).toEqual({ status: "checking" })
      await refresh
      expect(controller.getSnapshot()).toEqual({
        status: status === 503 ? "unavailable" : "anonymous",
      })
      expect(dependencies.clearPrivateData).toHaveBeenCalledTimes(2)
    }
  )

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
