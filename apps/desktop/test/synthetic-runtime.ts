import { randomUUID } from "node:crypto"
import type {
  AgentRuntime,
  RuntimeEvent,
  RuntimeResponse,
  RuntimeStart,
  RuntimeContinuation,
} from "@workspace/core"
/** Test entry only. Never imported by the production entry/worker. No model, tools or account. */
export class SyntheticRuntime implements AgentRuntime {
  private listeners = new Set<(event: RuntimeEvent) => void>()
  private run?: RuntimeStart
  private responses = new Set<string>()
  private failedStop = false
  private waiting: (() => void)[] = []
  async probe() {
    return {
      providerId: "synthetic-test-only",
      available: true,
      capabilities: {
        nativeDelegation: true,
        structuredQuestions: true,
        resume: true,
      },
    }
  }
  async listSkills(path: string) {
    return [
      { name: "Synthetic test skill", path: `${path}/synthetic/SKILL.md` },
    ]
  }
  subscribe(listener: (event: RuntimeEvent) => void) {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }
  private emit(body: RuntimeEvent) {
    for (const listener of this.listeners) listener(body)
  }
  private event(runId: string) {
    return { runId, id: randomUUID(), at: new Date().toISOString() }
  }
  async start(run: RuntimeStart) {
    this.run = run
    this.failedStop = false
    this.responses.clear()
    this.emit({ ...this.event(run.runId), type: "run-started" })
    this.emit({
      ...this.event(run.runId),
      type: "agent-observed",
      agentId: "synthetic-child",
      parentId: run.rootAgentId,
    })
    if (run.task.includes("hold")) return
    for (const id of ["synthetic-first", "synthetic-second"])
      this.emit({
        ...this.event(run.runId),
        type: "question-opened",
        question: {
          id,
          agentId: "synthetic-child",
          prompt: `Synthetic question ${id}`,
          options: [{ id: "explicit", label: "Explicit synthetic choice" }],
          allowFreeText: true,
          blocking: true,
          status: "pending",
        },
      })
    this.emit({
      ...this.event(run.runId),
      type: "permission-requested",
      request: {
        id: "synthetic-permission",
        agentId: run.rootAgentId,
        description: "Synthetic permission: no tool will run",
        status: "pending",
      },
    })
    this.emit({
      ...this.event(run.runId),
      type: "message-delta",
      agentId: "synthetic-child",
      messageId: "synthetic-message",
      delta: "Temporary synthetic output",
    })
    this.emit({
      ...this.event(run.runId),
      type: "message-completed",
      message: {
        id: "synthetic-message",
        agentId: "synthetic-child",
        text: '<img src=x onerror="window.syntheticInjection=true"> Synthetic inert output',
      },
    })
  }
  async respond(response: RuntimeResponse) {
    this.responses.add(response.requestId)
    if (response.kind === "question") {
      if (
        !this.responses.has("synthetic-first") ||
        !this.responses.has("synthetic-second")
      )
        await new Promise<void>((resolve) => this.waiting.push(resolve))
      else {
        for (const resolve of this.waiting) resolve()
        this.waiting = []
      }
    }
    if (this.responses.size === 3 && this.run) {
      this.emit({
        ...this.event(this.run.runId),
        type: "agent-finished",
        agentId: "synthetic-child",
        outcome: "completed",
      })
      this.emit({ ...this.event(this.run.runId), type: "run-completed" })
    }
  }
  async continue(command: RuntimeContinuation) {
    this.emit({
      ...this.event(command.runId),
      type: "message-completed",
      message: {
        id: randomUUID(),
        agentId: command.agentId,
        text: "Synthetic steering received",
      },
    })
  }
  async interrupt() {
    if (this.run?.task.includes("retry stop") && !this.failedStop) {
      this.failedStop = true
      throw new Error("Synthetic stop confirmation not yet available")
    }
    for (const resolve of this.waiting) resolve()
    this.waiting = []
  }
  async close() {
    await this.interrupt()
  }
}
