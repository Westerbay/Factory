import type { ChildProcessWithoutNullStreams } from "node:child_process"
import { StringDecoder } from "node:string_decoder"
import { ExecutionError } from "@workspace/core"
import { z } from "zod"

const idSchema = z.union([z.string(), z.number().int()])
const envelope = z.object({
  id: idSchema.optional(),
  method: z.string().optional(),
  params: z.unknown().optional(),
  result: z.unknown().optional(),
  error: z.object({ code: z.number(), message: z.string() }).optional(),
})
export type RpcId = z.infer<typeof idSchema>
export interface RpcMessage {
  id?: RpcId
  method: string
  params?: unknown
}
interface Pending {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  timer: ReturnType<typeof setTimeout>
}
export interface TransportOptions {
  timeoutMs?: number
  maxFrameBytes?: number
  maxPending?: number
}
/** JSON-RPC over bounded newline-delimited UTF-8. Raw stderr/errors never escape this boundary. */
export class JsonRpcTransport {
  private readonly pending = new Map<RpcId, Pending>()
  private readonly listeners = new Set<(message: RpcMessage) => void>()
  private readonly disconnects = new Set<() => void>()
  private sequence = 0
  private buffer = ""
  private decoder = new StringDecoder("utf8")
  private stopped = false
  private exited = false
  private exitWaiters: (() => void)[] = []
  private readonly timeoutMs: number
  private readonly maxFrameBytes: number
  private readonly maxPending: number
  constructor(
    private readonly child: ChildProcessWithoutNullStreams,
    options: TransportOptions = {}
  ) {
    this.timeoutMs = options.timeoutMs ?? 15_000
    this.maxFrameBytes = options.maxFrameBytes ?? 2_000_000
    this.maxPending = options.maxPending ?? 100
    child.stdout.on("data", (chunk: Buffer) =>
      this.receive(this.decoder.write(chunk))
    )
    // Drain, but neither accumulate nor forward diagnostics containing account/tool data.
    child.stderr.on("data", () => {})
    child.on("error", () => this.fail())
    child.stdin.on("error", () => this.fail())
    child.on("close", () => {
      this.exited = true
      this.fail()
      for (const done of this.exitWaiters) done()
      this.exitWaiters = []
    })
  }
  onMessage(listener: (message: RpcMessage) => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }
  onDisconnect(listener: () => void): () => void {
    this.disconnects.add(listener)
    return () => this.disconnects.delete(listener)
  }
  request(method: string, params: unknown): Promise<unknown> {
    if (this.stopped || this.pending.size >= this.maxPending)
      return Promise.reject(
        new ExecutionError(
          "unavailable",
          "Codex connection unavailable or request limit reached"
        )
      )
    const id = ++this.sequence
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new ExecutionError("runtime", "Codex request timed out"))
        this.fail()
      }, this.timeoutMs)
      this.pending.set(id, { resolve, reject, timer })
      try {
        this.send({ id, method, params })
      } catch (error) {
        clearTimeout(timer)
        this.pending.delete(id)
        reject(error)
        this.fail()
      }
    })
  }
  notify(method: string, params?: unknown): void {
    this.send({ method, ...(params === undefined ? {} : { params }) })
  }
  respond(id: RpcId, result: unknown): void {
    this.send({ id, result })
  }
  reject(id: RpcId): void {
    this.send({
      id,
      error: { code: -32601, message: "Unsupported client request" },
    })
  }
  private send(value: unknown): void {
    if (this.stopped)
      throw new ExecutionError("unavailable", "Codex connection closed")
    const frame = JSON.stringify(value) + "\n"
    if (Buffer.byteLength(frame) > this.maxFrameBytes)
      throw new ExecutionError("runtime", "Codex frame limit exceeded")
    this.child.stdin.write(frame)
  }
  private receive(text: string): void {
    if (this.stopped) return
    this.buffer += text
    let newline = this.buffer.indexOf("\n")
    while (newline >= 0) {
      const line = this.buffer.slice(0, newline)
      this.buffer = this.buffer.slice(newline + 1)
      if (Buffer.byteLength(line) > this.maxFrameBytes) {
        this.fail()
        return
      }
      if (line.trim()) {
        try {
          const message = envelope.parse(JSON.parse(line))
          if (message.method) {
            for (const listener of this.listeners)
              listener({
                id: message.id,
                method: message.method,
                params: message.params,
              })
          } else if (message.id !== undefined) {
            const pending = this.pending.get(message.id)
            if (pending) {
              this.pending.delete(message.id)
              clearTimeout(pending.timer)
              if (message.error)
                pending.reject(
                  new ExecutionError("runtime", "Codex rejected the request")
                )
              else if (Object.hasOwn(message, "result"))
                pending.resolve(message.result)
              else {
                pending.reject(
                  new ExecutionError("runtime", "Invalid Codex response")
                )
                this.fail()
              }
            }
          } else this.fail()
        } catch {
          this.fail()
          return
        }
      }
      newline = this.buffer.indexOf("\n")
    }
    if (Buffer.byteLength(this.buffer) > this.maxFrameBytes) this.fail()
  }
  private fail(): void {
    if (this.stopped) return
    this.stopped = true
    this.buffer = ""
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer)
      pending.reject(new ExecutionError("unavailable", "Codex connection lost"))
    }
    this.pending.clear()
    for (const listener of this.disconnects) listener()
    if (!this.exited) this.child.kill("SIGTERM")
  }
  async close(): Promise<void> {
    this.fail()
    if (this.exited) return
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.child.kill("SIGKILL")
        const final = setTimeout(
          () =>
            reject(
              new ExecutionError(
                "runtime",
                "Codex process termination unconfirmed"
              )
            ),
          2_000
        )
        this.exitWaiters.push(() => {
          clearTimeout(final)
          resolve()
        })
      }, 2_000)
      this.exitWaiters.push(() => {
        clearTimeout(timer)
        resolve()
      })
    })
  }
}
