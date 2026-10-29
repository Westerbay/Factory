import { afterEach, describe, expect, it, vi } from "vitest"

import type { Session } from "../../lib/auth/session-api"
import type { SessionState } from "../../lib/auth/session-controller"
import {
  EnvironmentFailure,
  type Environment,
  type EnvironmentAction,
} from "./api"
import { EnvironmentController } from "./environment-controller"

const controllers: EnvironmentController[] = []
afterEach(() => {
  for (const controller of controllers.splice(0)) controller.dispose()
  vi.useRealTimers()
})

function setup(status: Environment["status"] = "running") {
  vi.useFakeTimers()
  const environment: Environment = { status }
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
    read: vi
      .fn<(signal: AbortSignal) => Promise<Environment>>()
      .mockImplementation(async () => environment),
    change: vi
      .fn<
        (
          action: EnvironmentAction,
          token: string,
          signal: AbortSignal
        ) => Promise<Environment>
      >()
      .mockResolvedValue(environment),
  }
  const controller = new EnvironmentController(dependencies)
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

describe("environment activity", () => {
  it("polls while visible but sends no keepalive or stop, even a day later", async () => {
    const { controller, dependencies } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(15_000)
    expect(dependencies.read).toHaveBeenCalledTimes(4)
    expect(dependencies.change).not.toHaveBeenCalled()
    dependencies.visible.mockReturnValue(false)
    controller.synchronize()
    await vi.advanceTimersByTimeAsync(86_400_000)
    expect(dependencies.read).toHaveBeenCalledTimes(4)
    expect(dependencies.change).not.toHaveBeenCalled()
    dependencies.visible.mockReturnValue(true)
    controller.synchronize()
    await vi.advanceTimersByTimeAsync(0)
    expect(controller.getSnapshot().environment?.status).toBe("running")
    expect(dependencies.change).not.toHaveBeenCalled()
  })

  it("allows an explicit stop while Docker is unavailable", async () => {
    const { controller, dependencies } = setup("unavailable")
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    await controller.stop()
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "stop",
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
  })

  it("only sends one start; closing the controller does not send a stop", async () => {
    const { controller, dependencies } = setup("not_created")
    dependencies.change.mockResolvedValue({ status: "starting" })
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    await controller.start()
    await vi.advanceTimersByTimeAsync(15_000)
    controller.dispose()
    await vi.advanceTimersByTimeAsync(86_400_000)
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "start",
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
  })

  it.each(["visible", "online"] as const)(
    "cancels requests and timers when %s becomes false, and rechecks on return",
    async (condition) => {
      const { controller, dependencies } = setup()
      let finish!: (environment: Environment) => void
      dependencies.read.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve
          })
      )
      controller.connect()
      const signal = dependencies.read.mock.calls[0]![0]
      dependencies[condition].mockReturnValue(false)
      controller.synchronize()
      expect(signal.aborted).toBe(true)
      finish({ status: "running" })
      await vi.advanceTimersByTimeAsync(120_000)
      expect(controller.getSnapshot().environment).toBeNull()
      expect(dependencies.read).toHaveBeenCalledTimes(1)
      expect(dependencies.change).not.toHaveBeenCalled()
      dependencies[condition].mockReturnValue(true)
      controller.synchronize()
      await vi.advanceTimersByTimeAsync(0)
      expect(dependencies.read).toHaveBeenCalledTimes(2)
      expect(controller.getSnapshot().environment?.status).toBe("running")
    }
  )

  it.each([
    "checking",
    "anonymous",
    "offline",
    "logout-failed",
    "unavailable",
  ] as const)(
    "aborts immediately on session %s, without stopping the server environment",
    async (status) => {
      const { controller, dependencies, setSession } = setup()
      let finish!: (environment: Environment) => void
      dependencies.read.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve
          })
      )
      controller.connect()
      const signal = dependencies.read.mock.calls[0]![0]
      setSession({ status })
      expect(signal.aborted).toBe(true)
      finish({ status: "running" })
      const reads = dependencies.read.mock.calls.length
      await vi.advanceTimersByTimeAsync(86_400_000)
      expect(controller.getSnapshot().environment).toBeNull()
      expect(dependencies.change).not.toHaveBeenCalled()
      expect(dependencies.read).toHaveBeenCalledTimes(reads)
    }
  )

  it("does not restore a previous user's response after a verified session change", async () => {
    const { controller, dependencies, session, setSession } = setup("stopped")
    let finish!: (environment: Environment) => void
    dependencies.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        })
    )
    controller.connect()
    setSession({
      status: "active",
      session: { ...session, user: { ...session.user, id: "other-member" } },
    })
    await vi.advanceTimersByTimeAsync(0)
    finish({ status: "running" })
    await vi.advanceTimersByTimeAsync(0)
    expect(controller.getSnapshot().environment?.status).toBe("stopped")
  })

  it("suppresses overlapping commands and ignores a read arriving after stop", async () => {
    const { controller, dependencies } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    let finishRead!: (environment: Environment) => void
    let finishStop!: (environment: Environment) => void
    dependencies.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishRead = resolve
        })
    )
    const refresh = controller.refresh()
    const signal = dependencies.read.mock.calls.at(-1)![0]
    dependencies.change.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishStop = resolve
        })
    )
    const stopping = controller.stop()
    expect(signal.aborted).toBe(true)
    await controller.stop()
    await controller.start()
    await controller.refresh()
    expect(dependencies.change).toHaveBeenCalledTimes(1)
    finishStop({ status: "stopping" })
    await stopping
    finishRead({ status: "running" })
    await refresh
    expect(controller.getSnapshot().environment?.status).toBe("stopping")
  })

  it("clears stale Running on read failure, and recovers through refresh", async () => {
    const { controller, dependencies } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    dependencies.change.mockClear()
    dependencies.read.mockRejectedValue(new Error("offline"))
    await vi.advanceTimersByTimeAsync(60_000)
    expect(controller.getSnapshot()).toMatchObject({
      environment: null,
      message: expect.any(String),
    })
    expect(dependencies.change).not.toHaveBeenCalled()
    dependencies.read.mockResolvedValue({ status: "stopped" })
    await controller.refresh()
    expect(controller.getSnapshot()).toMatchObject({
      environment: { status: "stopped" },
      message: null,
    })
  })

  it("reports a conflicting start without retrying start or claiming it started", async () => {
    const { controller, dependencies } = setup("not_created")
    dependencies.change.mockRejectedValue(
      new EnvironmentFailure(409, "conflict")
    )
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    await controller.start()
    expect(controller.getSnapshot()).toMatchObject({
      environment: null,
      message: expect.stringContaining("changed"),
    })
    await vi.advanceTimersByTimeAsync(120_000)
    expect(dependencies.change).toHaveBeenCalledTimes(1)
  })

  it("cancels on unmount and supports React's setup-cleanup-setup replay", async () => {
    const { controller, dependencies } = setup("starting")
    controller.connect()
    const signal = dependencies.read.mock.calls[0]![0]
    controller.dispose()
    expect(signal.aborted).toBe(true)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(dependencies.read).toHaveBeenCalledTimes(1)
    controller.connect()
    await vi.advanceTimersByTimeAsync(60_000)
    expect(dependencies.change).not.toHaveBeenCalled()
  })
})
