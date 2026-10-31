import { afterEach, describe, expect, it, vi } from "vitest"

import type { Session } from "../../src/lib/auth/session-api"
import type { SessionState } from "../../src/lib/auth/session-controller"
import {
  EnvironmentFailure,
  type Environment,
  type EnvironmentAction,
} from "../../src/features/environments/api"
import { EnvironmentController } from "../../src/features/environments/environment-controller"

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
    hasRecoverableRuns: vi
      .fn<(signal: AbortSignal) => Promise<boolean>>()
      .mockResolvedValue(false),
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
  it("retains Running while hidden and rechecks silently without reversing a stop", async () => {
    const { controller, dependencies } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    dependencies.visible.mockReturnValue(false)
    controller.synchronize()
    expect(controller.getSnapshot()).toMatchObject({
      environment: { status: "running" },
      paused: true,
    })
    await vi.advanceTimersByTimeAsync(30_000)
    expect(dependencies.read).toHaveBeenCalledTimes(1)
    let finish!: (environment: Environment) => void
    dependencies.read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        })
    )
    dependencies.visible.mockReturnValue(true)
    controller.synchronize()
    expect(controller.getSnapshot()).toMatchObject({
      environment: { status: "running" },
      pending: "read",
      paused: false,
    })
    finish({ status: "stopped" })
    await vi.advanceTimersByTimeAsync(0)
    expect(controller.getSnapshot().environment?.status).toBe("stopped")
    expect(dependencies.change).not.toHaveBeenCalled()
  })

  it("does not reset observations or cancel reads for session metadata updates", async () => {
    const { controller, dependencies, session, setSession } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
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
    await vi.advanceTimersByTimeAsync(5_000)
    expect(dependencies.read).toHaveBeenCalledTimes(2)
    expect(dependencies.change).not.toHaveBeenCalled()
  })

  it("does not retain a Running observation when a stop is interrupted by hiding the tab", async () => {
    const { controller, dependencies } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    let finish!: (environment: Environment) => void
    dependencies.change.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        })
    )
    const stop = controller.stop()
    dependencies.visible.mockReturnValue(false)
    controller.synchronize()
    expect(dependencies.change.mock.calls[0]![2].aborted).toBe(true)
    expect(controller.getSnapshot().environment).toBeNull()
    finish({ status: "stopped" })
    await stop
    dependencies.read.mockResolvedValue({ status: "stopped" })
    dependencies.visible.mockReturnValue(true)
    controller.synchronize()
    await vi.advanceTimersByTimeAsync(0)
    expect(controller.getSnapshot().environment?.status).toBe("stopped")
    expect(dependencies.change).toHaveBeenCalledTimes(1)
  })

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

  it.each(["visible", "online"] as const)(
    "retries a cancelled first initialization when %s returns",
    async (condition) => {
      const { controller, dependencies } = setup("not_created")
      let complete!: (environment: Environment) => void
      dependencies.change.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            complete = resolve
          })
      )
      dependencies.change.mockResolvedValue({
        status: "starting",
      })
      controller.connect()
      await vi.advanceTimersByTimeAsync(0)
      expect(dependencies.change).toHaveBeenCalledTimes(1)
      const firstSignal = dependencies.change.mock.calls[0]![2]
      dependencies[condition].mockReturnValue(false)
      controller.synchronize()
      expect(firstSignal.aborted).toBe(true)
      complete({ status: "starting" })
      dependencies[condition].mockReturnValue(true)
      controller.synchronize()
      await vi.advanceTimersByTimeAsync(0)
      expect(dependencies.change).toHaveBeenCalledTimes(2)
      expect(dependencies.change.mock.calls[1]![0]).toBe("initialize")
      expect(controller.getSnapshot().environment?.status).toBe("starting")
    }
  )

  it("starts a stopped environment once after checking recovery", async () => {
    const { controller, dependencies } = setup("stopped")
    controller.connect()
    await vi.advanceTimersByTimeAsync(15_000)
    expect(dependencies.hasRecoverableRuns).toHaveBeenCalledTimes(1)
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "start",
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
  })

  it("does not automatically repeat a failed initialization", async () => {
    const { controller, dependencies } = setup("not_created")
    dependencies.change.mockRejectedValue(new EnvironmentFailure(503))
    controller.connect()
    await vi.advanceTimersByTimeAsync(30_000)
    expect(dependencies.change).toHaveBeenCalledTimes(1)
    await controller.retry()
    expect(dependencies.change).toHaveBeenCalledTimes(2)
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

  it("initializes once; closing the controller does not send a stop", async () => {
    const { controller, dependencies } = setup("not_created")
    dependencies.change.mockResolvedValue({
      status: "starting",
    })
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    await controller.start()
    await vi.advanceTimersByTimeAsync(15_000)
    controller.dispose()
    await vi.advanceTimersByTimeAsync(86_400_000)
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "initialize",
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
    const refresh = controller.retry()
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
    await controller.retry()
    expect(dependencies.change).toHaveBeenCalledTimes(1)
    finishStop({ status: "stopped" })
    await stopping
    finishRead({ status: "running" })
    await refresh
    expect(controller.getSnapshot().environment?.status).toBe("stopped")
    expect(dependencies.change).toHaveBeenCalledTimes(1)
  })

  it("clears stale Running on read failure, and resumes through explicit retry", async () => {
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
    dependencies.read.mockResolvedValue({
      status: "stopped",
    })
    await controller.retry()
    expect(controller.getSnapshot()).toMatchObject({
      environment: { status: "running" },
      message: null,
    })
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "start",
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
  })

  it.each(["running", "starting", "stopping"] as const)(
    "does not reverse a later stop after entering while %s",
    async (status) => {
      const { controller, dependencies } = setup(status)
      controller.connect()
      await vi.advanceTimersByTimeAsync(0)
      dependencies.read.mockResolvedValue({ status: "stopped" })
      await vi.advanceTimersByTimeAsync(15_000)
      expect(controller.getSnapshot().environment?.status).toBe("stopped")
      expect(dependencies.hasRecoverableRuns).not.toHaveBeenCalled()
      expect(dependencies.change).not.toHaveBeenCalled()
    }
  )

  it("keeps an explicit stop across focus verification and route remount", async () => {
    const { controller, dependencies, session, setSession } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    dependencies.change.mockResolvedValue({ status: "stopped" })
    await controller.stop()
    dependencies.read.mockResolvedValue({ status: "stopped" })
    setSession({ status: "checking" })
    controller.dispose()
    setSession({ status: "active", session: { ...session } })
    controller.connect()
    await vi.advanceTimersByTimeAsync(30_000)
    expect(controller.getSnapshot().environment?.status).toBe("stopped")
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "stop",
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
    await controller.start()
    expect(dependencies.change.mock.calls.at(-1)?.[0]).toBe("start")
  })

  it("does not undo a stop whose response was lost", async () => {
    const { controller, dependencies } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    dependencies.change.mockRejectedValue(new EnvironmentFailure(503))
    await controller.stop()
    dependencies.read.mockResolvedValue({ status: "stopped" })
    controller.dispose()
    controller.connect()
    await vi.advanceTimersByTimeAsync(15_000)
    expect(dependencies.change).toHaveBeenCalledTimes(1)
    expect(controller.getSnapshot().environment?.status).toBe("stopped")
  })

  it("does not loop failed starts through polls, focus checks or remounts", async () => {
    const { controller, dependencies, session, setSession } = setup("stopped")
    dependencies.change.mockRejectedValue(new EnvironmentFailure(503))
    controller.connect()
    await vi.advanceTimersByTimeAsync(15_000)
    setSession({ status: "checking" })
    controller.dispose()
    setSession({ status: "active", session: { ...session } })
    controller.connect()
    await vi.advanceTimersByTimeAsync(15_000)
    expect(dependencies.change).toHaveBeenCalledTimes(1)
    expect(controller.getSnapshot()).toMatchObject({
      environment: { status: "stopped" },
      message: expect.stringContaining("unavailable"),
    })
    await controller.retry()
    expect(dependencies.change).toHaveBeenCalledTimes(2)
  })

  it("automatically starts again for a newly verified login session", async () => {
    const { controller, dependencies, session, setSession } = setup()
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    dependencies.read.mockResolvedValue({ status: "stopped" })
    setSession({ status: "checking" })
    setSession({
      status: "active",
      session: { ...session, csrfToken: "new-synthetic-session" },
    })
    await vi.advanceTimersByTimeAsync(0)
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "start",
      "new-synthetic-session",
      expect.any(AbortSignal)
    )
  })

  it("keeps recoverable work stopped until an explicit resume rechecks it", async () => {
    const { controller, dependencies } = setup("stopped")
    dependencies.hasRecoverableRuns.mockResolvedValue(true)
    controller.connect()
    await vi.advanceTimersByTimeAsync(15_000)
    expect(dependencies.change).not.toHaveBeenCalled()
    expect(dependencies.hasRecoverableRuns).toHaveBeenCalledTimes(1)
    expect(controller.getSnapshot()).toMatchObject({
      environment: { status: "stopped" },
      message: expect.stringContaining("needs recovery"),
    })
    dependencies.hasRecoverableRuns.mockResolvedValue(false)
    await controller.start()
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "start",
      "synthetic-csrf",
      expect.any(AbortSignal)
    )
  })

  it("fails closed when recovery status cannot be read", async () => {
    const { controller, dependencies } = setup("stopped")
    dependencies.hasRecoverableRuns.mockRejectedValue(new Error("offline"))
    controller.connect()
    await vi.advanceTimersByTimeAsync(15_000)
    expect(dependencies.change).not.toHaveBeenCalled()
    expect(dependencies.hasRecoverableRuns).toHaveBeenCalledTimes(1)
    expect(controller.getSnapshot()).toMatchObject({
      environment: { status: "stopped" },
      message: expect.stringContaining(
        "Recovery status could not be confirmed"
      ),
    })
    dependencies.hasRecoverableRuns.mockResolvedValue(false)
    await controller.retry()
    expect(dependencies.change).toHaveBeenCalledTimes(1)
  })

  it("ignores a recovery preflight completed after the user changes", async () => {
    const { controller, dependencies, session, setSession } = setup("stopped")
    let finish!: (recoveryNeeded: boolean) => void
    dependencies.hasRecoverableRuns.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        })
    )
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    const signal = dependencies.hasRecoverableRuns.mock.calls[0]![0]
    setSession({
      status: "active",
      session: {
        ...session,
        user: { ...session.user, id: "other-member" },
        csrfToken: "other-synthetic-session",
      },
    })
    expect(signal.aborted).toBe(true)
    await vi.advanceTimersByTimeAsync(0)
    finish(false)
    await vi.advanceTimersByTimeAsync(0)
    expect(dependencies.change).toHaveBeenCalledExactlyOnceWith(
      "start",
      "other-synthetic-session",
      expect.any(AbortSignal)
    )
  })

  it("does not repeat an interrupted start after visibility returns", async () => {
    const { controller, dependencies } = setup("stopped")
    let finish!: (environment: Environment) => void
    dependencies.change.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve
        })
    )
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    const signal = dependencies.change.mock.calls[0]![2]
    dependencies.visible.mockReturnValue(false)
    controller.synchronize()
    expect(signal.aborted).toBe(true)
    dependencies.visible.mockReturnValue(true)
    controller.synchronize()
    await vi.advanceTimersByTimeAsync(0)
    finish({ status: "running" })
    await vi.advanceTimersByTimeAsync(15_000)
    expect(dependencies.change).toHaveBeenCalledTimes(1)
    expect(controller.getSnapshot().environment?.status).toBe("stopped")
  })

  it("does not turn a cancelled initialization into a restart", async () => {
    const { controller, dependencies } = setup("not_created")
    dependencies.change.mockImplementationOnce(() => new Promise(() => {}))
    controller.connect()
    await vi.advanceTimersByTimeAsync(0)
    dependencies.visible.mockReturnValue(false)
    controller.synchronize()
    dependencies.read.mockResolvedValue({ status: "stopped" })
    dependencies.visible.mockReturnValue(true)
    controller.synchronize()
    await vi.advanceTimersByTimeAsync(15_000)
    expect(dependencies.change).toHaveBeenCalledTimes(1)
    expect(controller.getSnapshot().environment?.status).toBe("stopped")
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
