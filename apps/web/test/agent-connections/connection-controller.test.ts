import { afterEach, expect, it, vi } from "vitest"

import type { Session } from "../../src/lib/auth/session-api"
import type { SessionState } from "../../src/lib/auth/session-controller"
import {
  ConnectionFailure,
  canDisconnectConnection,
  type Connection,
  type ConnectionAction,
} from "../../src/features/agent-connections/api"
import { ConnectionController } from "../../src/features/agent-connections/connection-controller"

const controllers: ConnectionController[] = []
const disconnected: Connection = { provider: "codex", status: "disconnected" }
function connecting(lifetime = 60_000): Connection {
  return {
    provider: "codex",
    status: "connecting",
    challenge: {
      operationId: "1e1b1d41-4221-44b9-a8e1-d5eaf5210355",
      verificationUrl: "https://auth.openai.com/codex/device",
      userCode: "SYNTHETIC-CODE",
      expiresAt: new Date(Date.now() + lifetime).toISOString(),
    },
  }
}
afterEach(() => {
  for (const controller of controllers.splice(0)) controller.dispose()
  vi.useRealTimers()
})
function setup(initial = disconnected) {
  vi.useFakeTimers()
  const session: Session = {
    user: {
      id: "member",
      handle: "member",
      displayName: "Member",
      role: "member",
    },
    csrfToken: "synthetic-csrf",
    expiresAt: new Date(Date.now() + 3_600_000).toISOString(),
  }
  let auth: SessionState = { status: "active", session }
  const listeners = new Set<() => void>()
  const dependencies = {
    session: {
      getSnapshot: () => auth,
      subscribe: (listener: () => void) => {
        listeners.add(listener)
        return () => {
          listeners.delete(listener)
        }
      },
    },
    visible: vi.fn().mockReturnValue(true),
    online: vi.fn().mockReturnValue(true),
    available: vi.fn().mockReturnValue(true),
    read: vi
      .fn<(signal: AbortSignal) => Promise<Connection>>()
      .mockResolvedValue(initial),
    change: vi
      .fn<
        (
          action: ConnectionAction,
          token: string,
          signal: AbortSignal
        ) => Promise<Connection>
      >()
      .mockResolvedValue(connecting()),
  }
  const controller = new ConnectionController(dependencies)
  controllers.push(controller)
  return {
    controller,
    dependencies,
    session,
    setSession(state: SessionState) {
      auth = state
      for (const listener of listeners) listener()
    },
  }
}
async function settle() {
  await vi.advanceTimersByTimeAsync(0)
}

it("only polls pending sign-in every two seconds and stops once the account is confirmed", async () => {
  const { controller, dependencies } = setup()
  controller.connect()
  await settle()
  await vi.advanceTimersByTimeAsync(10_000)
  expect(dependencies.read).toHaveBeenCalledTimes(1)
  dependencies.read.mockResolvedValue(connecting())
  await controller.login()
  await vi.advanceTimersByTimeAsync(6_000)
  expect(dependencies.read).toHaveBeenCalledTimes(4)
  dependencies.read.mockResolvedValue({
    provider: "codex",
    status: "connected",
    account: { email: "synthetic@example.test", plan: "plus" },
  })
  await vi.advanceTimersByTimeAsync(12_000)
  expect(dependencies.read).toHaveBeenCalledTimes(5)
  expect(controller.getSnapshot().connection?.status).toBe("connected")
  expect(dependencies.change).toHaveBeenCalledTimes(1)
})

it.each(["online", "available"] as const)(
  "clears private state when %s is lost and resumes by GET only",
  async (condition) => {
    const { controller, dependencies } = setup(connecting())
    controller.connect()
    await settle()
    dependencies[condition].mockReturnValue(false)
    controller.synchronize()
    expect(controller.getSnapshot().connection).toBeNull()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(dependencies.read).toHaveBeenCalledTimes(1)
    dependencies[condition].mockReturnValue(true)
    controller.synchronize()
    await settle()
    expect(dependencies.read).toHaveBeenCalledTimes(2)
    expect(controller.getSnapshot().connection?.challenge?.userCode).toBe(
      "SYNTHETIC-CODE"
    )
    expect(dependencies.change).not.toHaveBeenCalled()
  }
)

it("preserves a connected account while hidden and during its resumed status read", async () => {
  const connected: Connection = {
    provider: "codex",
    status: "connected",
    account: { plan: "plus" },
  }
  const { controller, dependencies } = setup(connected)
  controller.connect()
  await settle()
  dependencies.visible.mockReturnValue(false)
  controller.synchronize()
  expect(controller.getSnapshot()).toMatchObject({
    connection: connected,
    paused: true,
  })
  await vi.advanceTimersByTimeAsync(10_000)
  expect(dependencies.read).toHaveBeenCalledTimes(1)
  let finish!: (connection: Connection) => void
  dependencies.read.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve
      })
  )
  dependencies.visible.mockReturnValue(true)
  controller.synchronize()
  expect(controller.getSnapshot()).toMatchObject({
    connection: connected,
    pending: "read",
    paused: false,
  })
  finish(disconnected)
  await settle()
  expect(controller.getSnapshot().connection).toEqual(disconnected)
  expect(dependencies.change).not.toHaveBeenCalled()
})

it("keeps the original device-code deadline while hidden without polling or logging in again", async () => {
  const { controller, dependencies } = setup(connecting(3_000))
  controller.connect()
  await settle()
  dependencies.visible.mockReturnValue(false)
  controller.synchronize()
  await vi.advanceTimersByTimeAsync(2_000)
  expect(controller.getSnapshot().connection?.challenge?.userCode).toBe(
    "SYNTHETIC-CODE"
  )
  await vi.advanceTimersByTimeAsync(1_000)
  expect(controller.getSnapshot()).toMatchObject({
    connection: { errorCode: "expired" },
    paused: true,
  })
  expect(controller.getSnapshot().connection?.challenge).toBeUndefined()
  expect(dependencies.read).toHaveBeenCalledTimes(1)
  expect(dependencies.change).not.toHaveBeenCalled()
})

it("does not restart provider observation for verified session metadata updates", async () => {
  const { controller, dependencies, session, setSession } = setup(connecting())
  controller.connect()
  await settle()
  const before = controller.getSnapshot()
  setSession({
    status: "active",
    session: {
      ...session,
      user: { ...session.user, displayName: "Updated name" },
    },
  })
  expect(controller.getSnapshot()).toBe(before)
  expect(dependencies.read).toHaveBeenCalledTimes(1)
  await vi.advanceTimersByTimeAsync(2_000)
  expect(dependencies.read).toHaveBeenCalledTimes(2)
})

it.each([
  "checking",
  "anonymous",
  "offline",
  "unavailable",
  "logout-failed",
] as const)(
  "aborts immediately on %s and ignores an old response",
  async (status) => {
    const { controller, dependencies, setSession } = setup(connecting())
    controller.connect()
    await settle()
    let finish!: (connection: Connection) => void
    dependencies.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        })
    )
    const refresh = controller.refresh()
    const signal = dependencies.read.mock.calls.at(-1)![0]
    setSession({ status })
    expect(signal.aborted).toBe(true)
    expect(controller.getSnapshot().connection).toBeNull()
    finish(connecting())
    await refresh
    await vi.advanceTimersByTimeAsync(10_000)
    expect(controller.getSnapshot().connection).toBeNull()
    expect(dependencies.read).toHaveBeenCalledTimes(2)
    expect(dependencies.change).not.toHaveBeenCalled()
  }
)

it("does not restore codes from a previous verified account", async () => {
  const { controller, dependencies, setSession, session } = setup()
  let finish!: (connection: Connection) => void
  dependencies.read.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve
      })
  )
  controller.connect()
  setSession({
    status: "active",
    session: { ...session, user: { ...session.user, id: "other-user" } },
  })
  await settle()
  finish(connecting())
  await settle()
  expect(controller.getSnapshot().connection).toEqual(disconnected)
  expect(dependencies.change).not.toHaveBeenCalled()
})

it("sends one login and cancels the exact operation; late reads cannot restore its code", async () => {
  const { controller, dependencies } = setup()
  controller.connect()
  await settle()
  let finishLogin!: (connection: Connection) => void
  dependencies.change.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishLogin = resolve
      })
  )
  const login = controller.login()
  await controller.login()
  expect(dependencies.change).toHaveBeenCalledTimes(1)
  const pending = connecting()
  finishLogin(pending)
  await login
  let finishRead!: (connection: Connection) => void
  dependencies.read.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishRead = resolve
      })
  )
  const read = controller.refresh()
  dependencies.change.mockResolvedValue({
    provider: "codex",
    status: "error",
    errorCode: "cancelled",
  })
  await controller.cancelLogin()
  expect(dependencies.change).toHaveBeenLastCalledWith(
    { type: "cancel", operationId: pending.challenge!.operationId },
    "synthetic-csrf",
    expect.any(AbortSignal)
  )
  finishRead(pending)
  await read
  expect(controller.getSnapshot().connection?.challenge).toBeUndefined()
  expect(controller.getSnapshot().connection?.errorCode).toBe("cancelled")
})

it("expires a code while its status request is stalled and discards the late response", async () => {
  const { controller, dependencies } = setup(connecting(3_000))
  controller.connect()
  await settle()
  let finish!: (connection: Connection) => void
  dependencies.read.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve
      })
  )
  await vi.advanceTimersByTimeAsync(3_000)
  expect(dependencies.read.mock.calls.at(-1)![0].aborted).toBe(true)
  expect(controller.getSnapshot().connection?.errorCode).toBe("expired")
  expect(controller.getSnapshot().connection?.challenge).toBeUndefined()
  finish(connecting())
  await settle()
  expect(controller.getSnapshot().connection?.errorCode).toBe("expired")
  expect(dependencies.change).not.toHaveBeenCalled()
})

it("shows conflicts without retrying sign-in and drops uncertain account data", async () => {
  const { controller, dependencies } = setup()
  controller.connect()
  await settle()
  dependencies.change.mockRejectedValue(new ConnectionFailure(409))
  await controller.login()
  await vi.advanceTimersByTimeAsync(10_000)
  expect(controller.getSnapshot().connection).toBeNull()
  expect(controller.getSnapshot().message).toContain("Another sign-in")
  expect(dependencies.change).toHaveBeenCalledTimes(1)
})

it("disconnects only explicitly, with CSRF, and does not retain a connected badge after failure", async () => {
  const { controller, dependencies } = setup({
    provider: "codex",
    status: "connected",
    account: { email: null, plan: "plus" },
  })
  controller.connect()
  await settle()
  dependencies.change.mockRejectedValue(new Error("private provider data"))
  await controller.logout()
  expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
    { type: "logout" },
    "synthetic-csrf",
    expect.any(AbortSignal)
  )
  expect(controller.getSnapshot().connection).toBeNull()
  expect(controller.getSnapshot().message).not.toContain(
    "private provider data"
  )
  controller.dispose()
  await vi.advanceTimersByTimeAsync(10_000)
  expect(dependencies.change).toHaveBeenCalledTimes(1)
})

it.each(["unavailable", "error"] as const)(
  "allows explicit local logout while proxy account status is %s",
  async (status) => {
    const { controller, dependencies } = setup({
      provider: "codex",
      status,
      errorCode: "unavailable",
    })
    controller.connect()
    await settle()
    expect(canDisconnectConnection(controller.getSnapshot().connection)).toBe(
      true
    )
    expect(dependencies.change).not.toHaveBeenCalled()
    dependencies.change.mockResolvedValue(disconnected)
    await controller.logout()
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      { type: "logout" },
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
    expect(controller.getSnapshot().connection).toEqual(disconnected)
    expect(canDisconnectConnection(controller.getSnapshot().connection)).toBe(
      false
    )
  }
)
