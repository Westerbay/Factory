import { ExecutionError, isTerminal } from "../../core/dist/index.js"

export class MemoryRunStore {
  runs = new Map()
  events = new Map()
  failNextWrite = false
  writes = 0
  write() {
    this.writes++
    if (this.failNextWrite) {
      this.failNextWrite = false
      throw new Error("disk unavailable")
    }
  }
  async create(run, event) {
    this.write()
    if ([...this.runs.values()].some((item) => !isTerminal(item.status)))
      throw new ExecutionError("active-run", "An active root already exists")
    if (this.runs.has(run.id))
      throw new ExecutionError("invalid-transition", "Duplicate run ID")
    this.runs.set(run.id, structuredClone(run))
    this.events.set(run.id, [structuredClone(event)])
  }
  async commit(expectedVersion, run, event) {
    this.write()
    if (
      this.runs.get(run.id)?.version !== expectedVersion ||
      this.events.get(run.id).some((item) => item.id === event.id)
    )
      throw new ExecutionError("invalid-transition", "Conflicting event commit")
    this.runs.set(run.id, structuredClone(run))
    this.events.get(run.id).push(structuredClone(event))
  }
  async getActive() {
    return structuredClone(
      [...this.runs.values()].find((run) => !isTerminal(run.status))
    )
  }
  async get(id) {
    const run = this.runs.get(id)
    return run && structuredClone(run)
  }
  async findEvent(runId, eventId) {
    return structuredClone(
      this.events.get(runId)?.find((event) => event.id === eventId)
    )
  }
  async list(offset, limit) {
    const ordered = [...this.runs.values()].toReversed()
    return {
      runs: structuredClone(ordered.slice(offset, offset + limit)),
      nextOffset: ordered.length > offset + limit ? offset + limit : undefined,
    }
  }
  async history(runId, afterSequence, limit) {
    const events = this.events.get(runId) ?? []
    return {
      events: structuredClone(
        events.slice(afterSequence, afterSequence + limit)
      ),
      nextSequence:
        events.length > afterSequence + limit
          ? afterSequence + limit
          : undefined,
    }
  }
}
export class DeterministicRuntime {
  listeners = new Set()
  starts = []
  responses = []
  continuations = []
  stops = []
  capabilities = {
    nativeDelegation: true,
    structuredQuestions: true,
    resume: true,
  }
  startHook
  respondHook
  interruptHook
  closed = false
  sequence = 0
  async probe() {
    return {
      providerId: "test",
      available: true,
      capabilities: { ...this.capabilities },
    }
  }
  async listSkills() {
    return []
  }
  async start(command) {
    this.starts.push(command)
    if (this.startHook) return this.startHook(command)
    this.emit(command.runId, { type: "run-started" })
  }
  async respond(response) {
    this.responses.push(response)
    await this.respondHook?.(response)
  }
  async continue(command) {
    this.continuations.push(command)
  }
  async interrupt(runId) {
    this.stops.push(runId)
    await this.interruptHook?.(runId)
  }
  subscribe(listener) {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }
  async close() {
    this.closed = true
  }
  event(runId, body) {
    return {
      ...body,
      runId,
      id: `provider-${++this.sequence}`,
      at: "2026-10-26T10:00:00.000Z",
    }
  }
  emit(runId, body) {
    const event = this.event(runId, body)
    for (const listener of this.listeners) listener(event)
    return event
  }
}
export function deferred() {
  let resolve
  let reject
  const promise = new Promise((success, failure) => {
    resolve = success
    reject = failure
  })
  return { promise, resolve, reject }
}
