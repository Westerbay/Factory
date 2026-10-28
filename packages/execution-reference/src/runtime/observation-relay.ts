import type { DesktopEvent } from "@workspace/contracts"
const FLUSH_INTERVAL_MS = 40
const MAX_QUEUED_EVENTS = 128
const MAX_DELTA_CHARACTERS = 262_144
/** Coalesce transient bursts before IPC; snapshots replace skipped intermediate projections. */
export class ObservationRelay {
  private queue: DesktopEvent[] = []
  private deltaCharacters = 0
  private timer?: ReturnType<typeof setTimeout>
  private closed = false
  private overflow = false
  constructor(private send: (event: DesktopEvent) => void) {}
  publish = (event: DesktopEvent) => {
    if (this.closed) return
    if (event.type === "run-state") {
      this.queue = this.queue.filter(
        (queued) =>
          queued.type !== "run-state" || queued.run.id !== event.run.id
      )
      const completed = new Set(event.run.messages.map((message) => message.id))
      this.queue = this.queue.filter(
        (queued) =>
          queued.type !== "message-delta" ||
          queued.runId !== event.run.id ||
          !completed.has(queued.messageId)
      )
    }
    if (event.type === "message-delta") {
      if (this.deltaCharacters + event.delta.length > MAX_DELTA_CHARACTERS) {
        this.overflow = true
        return
      }
      this.deltaCharacters += event.delta.length
    }
    if (this.queue.length >= MAX_QUEUED_EVENTS) {
      // Keep a bounded outbox even if the renderer cannot consume a provider burst.
      const expendable = this.queue.findIndex(
        (queued) =>
          queued.type === "message-delta" || queued.type === "run-event"
      )
      this.queue.splice(expendable < 0 ? 0 : expendable, 1)
      this.overflow = true
    }
    this.queue.push(event)
    this.timer ??= setTimeout(() => this.flush(), FLUSH_INTERVAL_MS)
  }
  flush() {
    clearTimeout(this.timer)
    this.timer = undefined
    const queue = this.queue
    this.queue = []
    this.deltaCharacters = 0
    for (const event of queue) this.send(event)
    if (this.overflow) {
      this.overflow = false
      this.send({
        type: "diagnostic",
        code: "runtime",
        message:
          "Live updates were coalesced under load. Refresh the execution and durable events for authoritative history.",
      })
    }
  }
  close() {
    this.flush()
    this.closed = true
  }
}
