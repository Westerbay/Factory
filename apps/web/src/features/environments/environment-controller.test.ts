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
  let environment: Environment = {
    status,
    idleExpiresAt: new Date(Date.now() + 900_000).toISOString(),
  }
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
      .mockImplementation(async () => {
        environment = {
          ...environment,
          idleExpiresAt: new Date(Date.now() + 900_000).toISOString(),
        }
        return environment
      }),
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
  it.each([null, "2026-10-28T21:00:00Z"])(
    "queues a stop during an outage only when an idle lease exists (%s)",
    async (idleExpiresAt) => {
      const { controller, dependencies } = setup("unavailable")
      dependencies.read.mockResolvedValue({
        status: "unavailable",
        idleExpiresAt,
      })
      controller.connect()
      await vi.advanceTimersByTimeAsync(0)
      await controller.stop()
      expect(dependencies.change).toHaveBeenCalledTimes(idleExpiresAt ? 1 : 0)
      if (idleExpiresAt)
        expect(dependencies.change).toHaveBeenCalledWith(
          "stop",
          "synthetic-csrf",
          expect.any(AbortSignal)
        )
    }
  )

  it("renews on arrival, then every sixty seconds without five-second polls postponing it", async () => {
    const { controller, dependencies } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(59_999)
    expect(dependencies.read).toHaveBeenCalledTimes(12)
    expect(dependencies.change).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(dependencies.change).toHaveBeenCalledTimes(2)
    expect(dependencies.change).toHaveBeenLastCalledWith(
      "heartbeat",
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
  })

  it("renews immediately when returning with thirty seconds left on a fifteen-minute lease", async () => {
    const { controller, dependencies } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    dependencies.change.mockClear()
    dependencies.visible.mockReturnValue(false)
    controller.synchronize()
    await vi.advanceTimersByTimeAsync(870_000)
    expect(dependencies.change).not.toHaveBeenCalled()
    dependencies.visible.mockReturnValue(true)
    controller.synchronize()
    await vi.advanceTimersByTimeAsync(0)
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "heartbeat",
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
    expect(
      Date.parse(controller.getSnapshot().environment!.idleExpiresAt!)
    ).toBe(Date.now() + 900_000)
  })

  it("maintains a one-minute lease with margin despite continuous polling", async () => {
    const { controller, dependencies } = setup("starting")
    let expiresAt = Date.now() + 60_000
    const activeEnvironment = (): Environment => ({
      status: "starting",
      idleExpiresAt: new Date(expiresAt).toISOString(),
    })
    dependencies.read.mockImplementation(async () => activeEnvironment())
    dependencies.change.mockImplementation(async () => {
      expect(Date.now()).toBeLessThan(expiresAt)
      expiresAt = Date.now() + 60_000
      return activeEnvironment()
    })
    controller.connect()
    await vi.advanceTimersByTimeAsync(149_999)
    expect(dependencies.change).toHaveBeenCalledTimes(3)
    await vi.advanceTimersByTimeAsync(1)
    expect(dependencies.change).toHaveBeenCalledTimes(4)
    expect(expiresAt - Date.now()).toBe(60_000)
  })

  it.each([null, "expired", "invalid"])(
    "does not renew a running environment with a %s lease",
    async (lease) => {
      const { controller, dependencies } = setup()
      const idleExpiresAt =
        lease === "expired" ? new Date(Date.now()).toISOString() : lease
      dependencies.read.mockResolvedValue({ status: "running", idleExpiresAt })
      controller.connect()
      await vi.advanceTimersByTimeAsync(120_000)
      expect(dependencies.change).not.toHaveBeenCalled()
    }
  )

  it("schedules renewal before expiry after Start establishes a new lease", async () => {
    const { controller, dependencies } = setup("not_created")
    let expiresAt = Date.now() + 60_000
    dependencies.change.mockImplementation(async () => {
      expiresAt = Date.now() + 60_000
      return {
        status: "starting",
        idleExpiresAt: new Date(expiresAt).toISOString(),
      }
    })
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    await controller.start()
    dependencies.read.mockImplementation(async () => ({
      status: "starting",
      idleExpiresAt: new Date(expiresAt).toISOString(),
    }))
    await vi.advanceTimersByTimeAsync(49_999)
    expect(dependencies.change).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(dependencies.change).toHaveBeenLastCalledWith(
      "heartbeat",
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
  })

  it("does not busy-loop when renewal returns an unchanged, nearly expired lease", async () => {
    const { controller, dependencies } = setup()
    const environment: Environment = {
      status: "running",
      idleExpiresAt: new Date(Date.now() + 500).toISOString(),
    }
    dependencies.read.mockResolvedValue(environment)
    dependencies.change.mockResolvedValue(environment)
    controller.connect()
    await vi.advanceTimersByTimeAsync(5_000)
    expect(dependencies.change).toHaveBeenCalledTimes(1)
  })

  it.each([
    "not_created",
    "stopped",
    "stopping",
    "error",
    "unavailable",
  ] as const)("does not renew an environment in %s", async (status) => {
    const { controller, dependencies } = setup(status)
    controller.connect()
    await vi.advanceTimersByTimeAsync(120_000)
    expect(dependencies.change).not.toHaveBeenCalled()
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
    "aborts immediately on session %s, including a pending heartbeat",
    async (status) => {
      const { controller, dependencies, setSession } = setup()
      let finish!: (environment: Environment) => void
      dependencies.change.mockImplementation(
        () =>
          new Promise((resolve) => {
            finish = resolve
          })
      )
      controller.connect()
      await vi.advanceTimersByTimeAsync(60_000)
      const signal = dependencies.change.mock.calls[0]![2]
      setSession({ status })
      expect(signal.aborted).toBe(true)
      finish({ status: "running" })
      const reads = dependencies.read.mock.calls.length
      await vi.advanceTimersByTimeAsync(120_000)
      expect(controller.getSnapshot().environment).toBeNull()
      expect(dependencies.change).toHaveBeenCalledTimes(1)
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

  it("suppresses overlapping commands and ignores a heartbeat arriving after stop", async () => {
    const { controller, dependencies } = setup()
    let finishHeartbeat!: (environment: Environment) => void
    let finishStop!: (environment: Environment) => void
    dependencies.change
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finishHeartbeat = resolve
          })
      )
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finishStop = resolve
          })
      )
    controller.connect()
    await vi.advanceTimersByTimeAsync(60_000)
    const stopping = controller.stop()
    expect(dependencies.change.mock.calls[0]![2].aborted).toBe(true)
    await controller.stop()
    await controller.start()
    await controller.refresh()
    expect(dependencies.change).toHaveBeenCalledTimes(2)
    finishStop({ status: "stopping" })
    await stopping
    finishHeartbeat({ status: "running" })
    await vi.advanceTimersByTimeAsync(0)
    expect(controller.getSnapshot().environment?.status).toBe("stopping")
  })

  it("clears stale Running on read failure, suppresses heartbeat, and recovers through refresh", async () => {
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

  it("reports capacity refusal without retrying start or claiming it started", async () => {
    const { controller, dependencies } = setup("not_created")
    dependencies.change.mockRejectedValue(
      new EnvironmentFailure(409, "capacity_reached")
    )
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    await controller.start()
    expect(controller.getSnapshot()).toMatchObject({
      environment: null,
      message: expect.stringContaining("slots"),
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
    expect(dependencies.change).toHaveBeenCalledTimes(2)
  })
})
