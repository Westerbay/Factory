import { afterEach, describe, expect, it, vi } from "vitest"

import { ApiFailure } from "../../lib/api/client"
import type { SessionState } from "../../lib/auth/session-controller"
import type { requestAttachment } from "./api"
import { TerminalController } from "./terminal-controller"

class Socket {
  static OPEN = 1
  readyState = 0
  bufferedAmount = 0
  binaryType = "blob"
  onopen: ((event: Event) => void) | null = null
  onmessage: ((event: MessageEvent<unknown>) => void) | null = null
  onerror: (() => void) | null = null
  onclose: (() => void) | null = null
  sent: Record<string, unknown>[] = []
  closed = false
  constructor() {
    sockets.push(this)
  }
  send(data: string) {
    this.sent.push(JSON.parse(data))
  }
  close() {
    this.closed = true
    this.readyState = 3
  }
  open() {
    this.readyState = 1
    this.onopen?.(new Event("open"))
  }
  receive(data: unknown) {
    this.onmessage?.(new MessageEvent("message", { data }))
  }
  ready() {
    this.open()
    this.receive(JSON.stringify({ type: "ready" }))
  }
}
const sockets: Socket[] = []
const controllers: TerminalController[] = []
afterEach(() => {
  for (const controller of controllers.splice(0)) controller.dispose()
  sockets.length = 0
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

function setup() {
  vi.useFakeTimers()
  vi.stubGlobal("WebSocket", Socket)
  let auth: SessionState = {
    status: "active",
    session: {
      user: {
        id: "member",
        handle: "member",
        displayName: "Member",
        role: "member",
      },
      csrfToken: "synthetic-csrf",
      expiresAt: new Date(Date.now() + 3600_000).toISOString(),
    },
  }
  const listeners = new Set<() => void>()
  const pending: (() => void)[] = []
  const dependencies = {
    session: {
      getSnapshot: () => auth,
      subscribe: (listener: () => void) => {
        listeners.add(listener)
        return () => {
          listeners.delete(listener)
        }
      },
      authorizationFailed: vi.fn(),
    },
    request: vi.fn<typeof requestAttachment>().mockResolvedValue({
      ticket: "synthetic-ticket",
      expiresAt: new Date(Date.now() + 30_000).toISOString(),
    }),
    openSocket: () => new WebSocket("ws://fixture.invalid"),
    display: {
      write: vi.fn((_data: Uint8Array, done: () => void) => {
        pending.push(done)
      }),
      reset: vi.fn(),
    },
  }
  const controller = new TerminalController(dependencies)
  controllers.push(controller)
  return {
    controller,
    dependencies,
    pending,
    changeSession(state: SessionState) {
      auth = state
      for (const listener of listeners) listener()
    },
  }
}

describe("terminal attachment", () => {
  it("keeps the transport during a same-session refresh but closes it on token rotation", async () => {
    const { controller, dependencies, changeSession } = setup()
    await controller.connect()
    const socket = sockets[0]!
    socket.ready()
    const auth = dependencies.session.getSnapshot()
    if (auth.status !== "active") throw new Error("Expected active fixture")
    changeSession({ status: "active", session: { ...auth.session } })
    expect(socket.closed).toBe(false)
    expect(controller.getSnapshot().status).toBe("connected")
    controller.inputText("pending input")
    changeSession({
      status: "active",
      session: { ...auth.session, csrfToken: "rotated-synthetic-token" },
    })
    expect(socket.closed).toBe(true)
    expect(controller.getSnapshot().status).toBe("disconnected")
    await vi.advanceTimersByTimeAsync(20)
    expect(socket.sent.some((frame) => frame.type === "input")).toBe(false)
    expect(dependencies.display.reset).toHaveBeenCalledTimes(2)
  })

  it("sends the ticket only as the first frame and acknowledges output after display processing", async () => {
    const { controller, pending } = setup()
    controller.resize(100, 32)
    await controller.connect()
    const socket = sockets[0]!
    socket.ready()
    expect(socket.sent).toEqual([
      { type: "authenticate", ticket: "synthetic-ticket" },
      { type: "resize", cols: 100, rows: 32 },
    ])
    socket.receive(new Uint8Array([65, 66, 67]).buffer)
    expect(socket.sent).toHaveLength(2)
    pending.shift()!()
    expect(socket.sent.at(-1)).toEqual({ type: "ack", bytes: 3 })
  })

  it("batches input and never replays pending keystrokes or old acknowledgements after reconnect", async () => {
    const { controller, pending } = setup()
    await controller.connect()
    sockets[0]!.ready()
    controller.inputText("do not replay")
    sockets[0]!.receive(new Uint8Array([65]).buffer)
    controller.disconnect()
    await controller.connect()
    sockets[1]!.ready()
    pending.shift()!()
    await vi.advanceTimersByTimeAsync(100)
    expect(sockets[1]!.sent).toHaveLength(2)
    controller.inputText("echo ")
    controller.inputText("ok\r")
    expect(sockets[1]!.sent).toHaveLength(2)
    await vi.advanceTimersByTimeAsync(20)
    expect(sockets[1]!.sent.at(-1)).toEqual({
      type: "input",
      data: "echo ok\r",
    })
  })

  it("chunks pasted unicode within the UTF-8 limit without breaking surrogate pairs", async () => {
    const { controller } = setup()
    await controller.connect()
    const socket = sockets[0]!
    socket.ready()
    const paste = "a" + "🙂".repeat(7000)
    controller.inputText(paste)
    await vi.advanceTimersByTimeAsync(20)
    const chunks = socket.sent
      .filter((frame) => frame.type === "input")
      .map((frame) => String(frame.data))
    expect(chunks.join("")).toBe(paste)
    for (const chunk of chunks) {
      expect(new TextEncoder().encode(chunk).byteLength).toBeLessThanOrEqual(
        16 * 1024
      )
      expect(new TextDecoder().decode(new TextEncoder().encode(chunk))).toBe(
        chunk
      )
    }
    const count = socket.sent.length
    controller.inputText("x".repeat(65 * 1024))
    await vi.advanceTimersByTimeAsync(20)
    expect(socket.sent).toHaveLength(count)
    expect(controller.getSnapshot().message).toContain("too large")
  })

  it("requires an explicit takeover after conflict instead of retrying automatically", async () => {
    const { controller, dependencies } = setup()
    dependencies.request.mockRejectedValueOnce(new ApiFailure(409))
    await controller.connect()
    expect(controller.getSnapshot().status).toBe("conflict")
    expect(sockets).toHaveLength(0)
    await controller.connect(true)
    expect(dependencies.request.mock.calls[1]?.[0]).toBe(true)
    expect(sockets).toHaveLength(1)
  })

  it("closes and clears private display on invalidation and discards a stale ticket response", async () => {
    const { controller, dependencies, changeSession } = setup()
    let resolve:
      | ((ticket: { ticket: string; expiresAt: string }) => void)
      | undefined
    dependencies.request.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done
        })
    )
    const opening = controller.connect()
    changeSession({ status: "anonymous" })
    resolve!({
      ticket: "late-ticket",
      expiresAt: new Date(Date.now() + 30_000).toISOString(),
    })
    await opening
    expect(sockets).toHaveLength(0)
    expect(dependencies.display.reset).toHaveBeenCalledTimes(2)
    expect(controller.getSnapshot().status).toBe("disconnected")
  })

  it("rejects unexpected output before ready and oversized output without passing it to xterm", async () => {
    for (const ready of [false, true]) {
      const { controller, dependencies } = setup()
      await controller.connect()
      const socket = sockets.at(-1)!
      if (ready) socket.ready()
      else socket.open()
      socket.receive(new Uint8Array(ready ? 16 * 1024 + 1 : 1).buffer)
      expect(controller.getSnapshot().status).toBe("error")
      expect(socket.closed).toBe(true)
      expect(dependencies.display.write).not.toHaveBeenCalled()
    }
  })
})
