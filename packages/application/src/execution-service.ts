import {
  ExecutionError,
  isTerminal,
  reduceEvent,
  requireActive,
  requireAgent,
  type AgentRuntime,
  type RunStore,
  type Run,
  type RunEvent,
  type RuntimeEvent,
  type EventBody,
  type StartRun,
  type RuntimeResponse,
  type QuestionAnswer,
  type PermissionDecision,
  type ContinuationReference,
} from "@workspace/core"

import { sameEventContent } from "./event-equality.js"

export interface ExecutionDependencies {
  runtime: AgentRuntime
  store: RunStore
  createId: () => string
  now: () => string
}
export type ExecutionObservation =
  | { durable: true; run: Run; event: RunEvent }
  | { durable: false; event: RuntimeEvent }
export interface ExecutionDiagnostics {
  errors: ExecutionError[]
  persistenceFailure?: { runId: string; stopped: boolean }
}
/** One coordinator per runtime host. Store also enforces the active-root invariant atomically. */
export class ExecutionService {
  private queue: Promise<void> = Promise.resolve()
  private background: Promise<void>[] = []
  private errors: ExecutionError[] = []
  private persistenceFailure?: { runId: string; stopped: boolean }
  private closed = false
  private closing = false
  private activeRunId?: string
  private stopping = new Map<string, Promise<void>>()
  private pendingCompletions = new Map<string, RuntimeEvent>()
  private observers = new Set<(observation: ExecutionObservation) => void>()
  private readonly unsubscribe: () => void
  private readonly runtime: AgentRuntime
  private readonly store: RunStore
  private readonly createId: () => string
  private readonly now: () => string

  constructor(dependencies: ExecutionDependencies) {
    this.runtime = dependencies.runtime
    this.store = dependencies.store
    this.createId = dependencies.createId
    this.now = dependencies.now
    this.unsubscribe = this.runtime.subscribe((event) => {
      const delivery = this.acceptEvent(event).catch((error) => {
        this.recordError(error)
      })
      this.track(delivery)
    })
  }
  private track(operation: Promise<void>): void {
    this.background.push(operation)
    void operation.then(() => {
      this.background = this.background.filter((item) => item !== operation)
    })
  }
  private recordError(error: unknown): void {
    if (error instanceof ExecutionError) this.errors.push(error)
    else
      this.errors.push(
        new ExecutionError("runtime", "Runtime event processing failed", {
          cause: error,
        })
      )
  }
  diagnostics(): ExecutionDiagnostics {
    return {
      errors: [...this.errors],
      persistenceFailure: this.persistenceFailure && {
        ...this.persistenceFailure,
      },
    }
  }
  /** Drain queued callbacks and safety-stop attempts; diagnostics retain any background error. */
  async idle(): Promise<void> {
    do {
      await Promise.all(this.background)
      await this.queue
    } while (this.background.length > 0)
  }
  private serial<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.queue.then(operation)
    this.queue = result.then(
      () => undefined,
      () => undefined
    )
    return result
  }
  subscribe(listener: (observation: ExecutionObservation) => void): () => void {
    this.observers.add(listener)
    return () => {
      this.observers.delete(listener)
    }
  }
  private publish(observation: ExecutionObservation): void {
    for (const listener of this.observers) {
      try {
        listener(observation)
      } catch (cause) {
        this.errors.push(
          new ExecutionError("runtime", "Host observation listener failed", {
            cause,
          })
        )
      }
    }
  }
  private available(allowClosing = false): void {
    if (
      this.closed ||
      (!allowClosing && this.closing) ||
      this.persistenceFailure
    )
      throw new ExecutionError(
        "unavailable",
        "Execution host is closed or persistence is unavailable; restart after recovery"
      )
  }
  private event(runId: string, body: EventBody): RunEvent {
    return { ...body, id: this.createId(), runId, at: this.now() }
  }
  private async storage<T>(
    runId: string,
    operation: () => Promise<T>
  ): Promise<T> {
    try {
      return await operation()
    } catch (cause) {
      const expectedConflict =
        cause instanceof ExecutionError &&
        ["active-run", "invalid-transition", "ownership", "not-found"].includes(
          cause.code
        )
      if (expectedConflict) throw cause
      const error = new ExecutionError(
        "persistence",
        "Execution state could not be persisted or read",
        { cause }
      )
      this.errors.push(error)
      if (!this.persistenceFailure) {
        this.persistenceFailure = { runId, stopped: false }
        // Never await a provider inside the mutation queue: it can emit synchronous callbacks.
        const termination = this.activeRunId
          ? this.runtime.interrupt(this.activeRunId)
          : this.runtime.close()
        const stop = termination
          .then(() => {
            if (this.persistenceFailure) this.persistenceFailure.stopped = true
          })
          .catch((cause) => {
            this.errors.push(
              new ExecutionError(
                "runtime",
                "Safety stop failed after persistence loss",
                { cause }
              )
            )
          })
        this.track(stop)
      }
      throw error
    }
  }
  private async load(runId: string): Promise<Run> {
    const run = await this.storage(runId, () => this.store.get(runId))
    if (!run) throw new ExecutionError("not-found", "Run was not found")
    return run
  }
  private async commit(run: Run, event: RunEvent): Promise<Run> {
    const next = reduceEvent(run, event)
    await this.storage(run.id, () =>
      this.store.commit(run.version, next, event)
    )
    if (isTerminal(next.status)) this.activeRunId = undefined
    if (isTerminal(next.status) || next.status === "cancelling")
      this.pendingCompletions.delete(run.id)
    this.publish({ durable: true, run: next, event })
    return this.drainCompletion(next)
  }
  private async drainCompletion(run: Run): Promise<Run> {
    const completion = this.pendingCompletions.get(run.id)
    if (!completion || completion.type !== "run-completed") return run
    const unresolvedRequests =
      run.questions.some(
        (question) =>
          question.status === "pending" || question.status === "submitting"
      ) ||
      run.permissions.some(
        (request) =>
          request.status === "pending" || request.status === "submitting"
      )
    const activeChildren = run.agents.some(
      (agent) => agent.id !== run.rootAgentId && !isTerminal(agent.status)
    )
    if (unresolvedRequests || activeChildren) return run
    this.pendingCompletions.delete(run.id)
    return this.commit(run, completion)
  }
  async startRun(command: StartRun): Promise<Run> {
    return this.start(command)
  }
  private async start(
    command: StartRun,
    continuationOf?: ContinuationReference
  ): Promise<Run> {
    this.available()
    const probe = await this.runtime.probe()
    if (!probe.available || probe.providerId !== command.providerId)
      throw new ExecutionError(
        "unsupported",
        probe.reason ?? "Selected provider is unavailable"
      )
    if (continuationOf && !probe.capabilities.resume)
      throw new ExecutionError(
        "unsupported",
        "Provider cannot continue an existing conversation"
      )
    const started = await this.serial(async () => {
      this.available()
      if (!command.workspacePath.trim() || !command.task.trim())
        throw new ExecutionError(
          "invalid-answer",
          "Workspace and task are required"
        )
      const now = this.now()
      const rootAgentId = this.createId()
      const run: Run = {
        ...command,
        continuationOf,
        id: this.createId(),
        rootAgentId,
        status: "created",
        version: 1,
        createdAt: now,
        updatedAt: now,
        agents: [{ id: rootAgentId, status: "created" }],
        questions: [],
        permissions: [],
        messages: [],
      }
      await this.storage(run.id, () =>
        this.store.create(run, this.event(run.id, { type: "run-created" }))
      )
      this.activeRunId = run.id
      const effect = this.runtime.start({
        runId: run.id,
        rootAgentId: run.rootAgentId,
        workspacePath: run.workspacePath,
        task: run.task,
        skill: run.skill,
        continuationOf,
      })
      return { run, effect }
    })
    const { run, effect } = started
    try {
      await effect
    } catch (cause) {
      await this.startupFailure(run.id, cause)
    }
    await this.idle()
    this.available()
    return this.load(run.id)
  }
  async acceptEvent(event: RuntimeEvent): Promise<void> {
    return this.serial(async () => {
      this.available(true)
      const run = await this.load(event.runId)
      // Streaming fragments are transient, never represented as durable completed output.
      if (event.type === "message-delta") {
        requireActive(run)
        if (isTerminal(requireAgent(run, event.agentId).status))
          throw new ExecutionError(
            "invalid-transition",
            "Streaming agent is terminal"
          )
        this.publish({ durable: false, event })
        return
      }
      const existing = await this.storage(run.id, () =>
        this.store.findEvent(run.id, event.id)
      )
      if (existing) {
        if (!sameEventContent(existing, event))
          throw new ExecutionError(
            "invalid-transition",
            "Event ID was reused with different content"
          )
        return
      }
      const submitting =
        run.questions.some((question) => question.status === "submitting") ||
        run.permissions.some((request) => request.status === "submitting")
      if (event.type === "run-completed" && submitting) {
        const pending = this.pendingCompletions.get(run.id)
        if (pending && !sameEventContent(pending, event))
          throw new ExecutionError(
            "invalid-transition",
            "Conflicting provider completion events"
          )
        this.pendingCompletions.set(run.id, event)
        return
      }
      await this.commit(run, event)
    })
  }
  async answerQuestion(
    runId: string,
    agentId: string,
    questionId: string,
    answer: QuestionAnswer
  ): Promise<void> {
    const pending = await this.serial(async () => {
      this.available()
      const run = await this.load(runId)
      requireActive(run)
      requireAgent(run, agentId)
      const question = run.questions.find((item) => item.id === questionId)
      if (
        !question ||
        question.agentId !== agentId ||
        question.status !== "pending"
      )
        throw new ExecutionError(
          "stale-request",
          "Question is not pending for this agent"
        )
      const hasOption = answer.optionId !== undefined
      const hasText = answer.text !== undefined
      if (
        hasOption === hasText ||
        (hasOption &&
          !question.options.some((option) => option.id === answer.optionId)) ||
        (hasText && (!question.allowFreeText || !answer.text?.trim()))
      )
        throw new ExecutionError(
          "invalid-answer",
          "Supply one offered option or supported nonempty free-text answer"
        )
      await this.commit(
        run,
        this.event(runId, { type: "question-submitting", questionId })
      )
      const response: RuntimeResponse = {
        kind: "question",
        runId,
        agentId,
        requestId: questionId,
        answer,
      }
      return { response, effect: this.runtime.respond(response) }
    })
    await this.sendResponse(pending.response, pending.effect)
  }
  async answerPermission(
    runId: string,
    agentId: string,
    requestId: string,
    decision: PermissionDecision
  ): Promise<void> {
    const pending = await this.serial(async () => {
      this.available()
      const run = await this.load(runId)
      requireActive(run)
      requireAgent(run, agentId)
      const request = run.permissions.find((item) => item.id === requestId)
      if (
        !request ||
        request.agentId !== agentId ||
        request.status !== "pending"
      )
        throw new ExecutionError(
          "stale-request",
          "Permission is not pending for this agent"
        )
      await this.commit(
        run,
        this.event(runId, { type: "permission-submitting", requestId })
      )
      const response: RuntimeResponse = {
        kind: "permission",
        runId,
        agentId,
        requestId,
        decision,
      }
      return { response, effect: this.runtime.respond(response) }
    })
    await this.sendResponse(pending.response, pending.effect)
  }
  private async sendResponse(
    response: RuntimeResponse,
    effect: Promise<void>
  ): Promise<void> {
    try {
      await effect
    } catch (cause) {
      await this.runtimeFailure(
        response.runId,
        "Provider response failed; it will not be retried",
        cause
      )
    }
    await this.serial(async () => {
      this.available()
      const run = await this.load(response.runId)
      // A stop/disconnect/child exit may have invalidated the handle while the RPC was in flight.
      requireActive(run)
      const body: EventBody =
        response.kind === "question"
          ? {
              type: "question-resolved",
              questionId: response.requestId,
              answer: response.answer,
            }
          : {
              type: "permission-resolved",
              requestId: response.requestId,
              decision: response.decision,
            }
      await this.commit(run, this.event(run.id, body))
    })
  }
  async continueRun(
    runId: string,
    agentId: string,
    text: string
  ): Promise<Run> {
    const prior = await this.serial(async () => {
      this.available()
      const run = await this.load(runId)
      requireAgent(run, agentId)
      if (!text.trim())
        throw new ExecutionError(
          "invalid-answer",
          "Continuation cannot be empty"
        )
      return run
    })
    if (isTerminal(prior.status))
      return this.start(
        {
          workspacePath: prior.workspacePath,
          providerId: prior.providerId,
          skill: prior.skill,
          task: text,
        },
        { runId, agentId }
      )
    const probe = await this.runtime.probe()
    if (!probe.capabilities.resume)
      throw new ExecutionError(
        "unsupported",
        "Provider cannot continue an existing conversation"
      )
    const pending = await this.serial(async () => {
      this.available()
      const run = await this.load(runId)
      requireActive(run)
      await this.commit(
        run,
        this.event(runId, { type: "continuation-requested", agentId, text })
      )
      return { effect: this.runtime.continue({ runId, agentId, text }) }
    })
    try {
      await pending.effect
    } catch (cause) {
      await this.runtimeFailure(runId, "Provider continuation failed", cause)
    }
    return this.load(runId)
  }
  async cancelRun(runId: string): Promise<Run> {
    const pending = await this.serial(async () => {
      this.available(true)
      const run = await this.load(runId)
      if (isTerminal(run.status)) return undefined
      if (run.status !== "cancelling")
        await this.commit(
          run,
          this.event(runId, {
            type: "run-cancelling",
            reason: "Stopped by user",
          })
        )
      let effect = this.stopping.get(runId)
      if (!effect) {
        effect = this.runtime.interrupt(runId)
        this.stopping.set(runId, effect)
      }
      return { effect }
    })
    if (!pending) return this.load(runId)
    try {
      await pending.effect
    } catch (cause) {
      const error = new ExecutionError(
        "runtime",
        "Stop is unconfirmed; execution remains cancelling",
        { cause }
      )
      this.errors.push(error)
      this.stopping.delete(runId)
      throw error
    }
    await this.serial(async () => {
      this.available(true)
      const run = await this.load(runId)
      if (run.status === "cancelling")
        await this.commit(
          run,
          this.event(runId, {
            type: "run-interrupted",
            reason: "All owned runtime work stopped",
          })
        )
    })
    this.stopping.delete(runId)
    return this.load(runId)
  }
  private async startupFailure(runId: string, cause: unknown): Promise<never> {
    const error = new ExecutionError("runtime", "Provider startup failed", {
      cause,
    })
    this.errors.push(error)
    try {
      await this.runtime.interrupt(runId)
    } catch (stopCause) {
      this.errors.push(
        new ExecutionError("runtime", "Startup cleanup failed", {
          cause: stopCause,
        })
      )
      await this.serial(async () => {
        const run = await this.load(runId)
        if (!isTerminal(run.status) && run.status !== "cancelling")
          await this.commit(
            run,
            this.event(runId, {
              type: "run-cancelling",
              reason: "Startup cleanup unconfirmed",
            })
          )
      })
      throw error
    }
    await this.serial(async () => {
      this.available()
      const run = await this.load(runId)
      if (!isTerminal(run.status)) {
        const type =
          run.status === "cancelling" ? "run-interrupted" : "run-failed"
        await this.commit(
          run,
          this.event(runId, {
            type,
            reason: "Provider startup failed; owned work stopped",
          })
        )
      }
    })
    throw error
  }
  private async runtimeFailure(
    runId: string,
    message: string,
    cause: unknown
  ): Promise<never> {
    const error = new ExecutionError("runtime", message, { cause })
    this.errors.push(error)
    await this.cancelRun(runId)
    throw error
  }
  async getRun(runId: string): Promise<Run> {
    this.available()
    return this.load(runId)
  }
  async listRuns(offset = 0, limit = 50) {
    this.page(offset, limit)
    return this.storage("host", () => this.store.list(offset, limit))
  }
  async history(runId: string, afterSequence = 0, limit = 100) {
    this.page(afterSequence, limit)
    await this.load(runId)
    return this.storage(runId, () =>
      this.store.history(runId, afterSequence, limit)
    )
  }
  private page(offset: number, limit: number): void {
    if (
      !Number.isSafeInteger(offset) ||
      offset < 0 ||
      !Number.isSafeInteger(limit) ||
      limit < 1 ||
      limit > 200
    )
      throw new ExecutionError(
        "invalid-answer",
        "Invalid history/list page (limit 1..200)"
      )
  }
  async close(): Promise<void> {
    if (this.closed) return
    this.closing = true
    if (!this.persistenceFailure) {
      const active = await this.storage("host", () => this.store.getActive())
      if (active) await this.cancelRun(active.id)
    }
    await this.runtime.close()
    await this.idle()
    this.closed = true
    this.unsubscribe()
    this.observers.clear()
  }
}
