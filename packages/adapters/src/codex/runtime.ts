import { randomUUID } from "node:crypto"
import { isAbsolute } from "node:path"
import {
  ExecutionError,
  type AgentRuntime,
  type RuntimeEvent,
  type RuntimeProbe,
  type RuntimeStart,
  type RuntimeResponse,
  type RuntimeContinuation,
  type SkillReference,
  type EventBody,
} from "@workspace/core"
import { z } from "zod"
import { type CodexConnector } from "./process.js"
import {
  type JsonRpcTransport,
  type RpcMessage,
  type RpcId,
} from "./transport.js"
import * as protocol from "./protocol.js"
import type { InitializeParams } from "./generated/InitializeParams.js"
import type { GetAccountParams } from "./generated/v2/GetAccountParams.js"
import type { SkillsListParams } from "./generated/v2/SkillsListParams.js"
import type { TurnSteerParams } from "./generated/v2/TurnSteerParams.js"
import type { ToolRequestUserInputResponse } from "./generated/v2/ToolRequestUserInputResponse.js"

interface OwnedAgent {
  id: string
  threadId: string
  parentId?: string
  parentThreadId?: string
  turnId?: string
  previousTurnIds?: Set<string>
  done: boolean
  outcome?: "completed" | "failed" | "interrupted"
}
interface OwnedRun {
  id: string
  workspacePath: string
  rootId: string
  agents: Map<string, OwnedAgent>
  stopping: boolean
  terminal: boolean
  cleanupConfirmed: boolean
  rootOutcome?: "completed" | "failed" | "interrupted"
}
interface AnswerSlot {
  questionId: string
  providerId: string
  options: Map<string, string>
  freeText: boolean
  value?: string
  submitted: boolean
}
interface PendingRequest {
  id: RpcId
  run: OwnedRun
  agent: OwnedAgent
  kind: "question" | "permission"
  slots: AnswerSlot[]
  sent: boolean
  waiters: { resolve: () => void; reject: (error: Error) => void }[]
  timer?: ReturnType<typeof setTimeout>
}
export interface CodexRuntimeOptions {
  connect: CodexConnector
  responseTimeoutMs?: number
  stopTimeoutMs?: number
  createId?: () => string
  now?: () => string
}
/** Owns all provider IDs and request handles. No provider internals enter durable events. */
export class CodexRuntime implements AgentRuntime {
  private transport?: JsonRpcTransport
  private connecting?: Promise<void>
  private starting?: string
  private interruptedRuns = new Set<string>()
  private closing = false
  private closeInFlight?: Promise<void>
  private closed = false
  private ready = false
  private lost = false
  private failure?: Promise<void>
  private stopFailure?: ExecutionError
  private listeners = new Set<(event: RuntimeEvent) => void>()
  private runs = new Map<string, OwnedRun>()
  private threads = new Map<string, { run: OwnedRun; agent: OwnedAgent }>()
  private requests = new Map<RpcId, PendingRequest>()
  private handles = new Map<string, PendingRequest>()
  private messageIds = new Map<string, string>()
  private completedMessages = new Map<string, string>()
  private unknownEvents: RpcMessage[] = []
  private readonly createId: () => string
  private readonly now: () => string
  constructor(private readonly options: CodexRuntimeOptions) {
    this.createId = options.createId ?? randomUUID
    this.now = options.now ?? (() => new Date().toISOString())
  }
  subscribe(listener: (event: RuntimeEvent) => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }
  private emit(
    run: OwnedRun,
    body:
      | EventBody
      | {
          type: "message-delta"
          agentId: string
          messageId: string
          delta: string
        }
  ): void {
    // The narrowed union below prevents application-only event kinds at the provider boundary.
    const identity = { id: this.createId(), runId: run.id, at: this.now() }
    switch (body.type) {
      case "run-started":
      case "agent-observed":
      case "agent-finished":
      case "message-completed":
      case "question-opened":
      case "permission-requested":
      case "run-completed":
      case "run-failed":
      case "run-interrupted":
      case "provider-disconnected":
      case "message-delta":
        for (const listener of this.listeners)
          listener({ ...identity, ...body })
    }
  }
  private async connect(): Promise<void> {
    if (this.closed || this.closing)
      throw new ExecutionError("unavailable", "Codex runtime closed or closing")
    if (
      [...this.runs.values()].some(
        (run) => run.terminal && !run.cleanupConfirmed
      )
    )
      throw new ExecutionError(
        "runtime",
        "Owned native cleanup unconfirmed; reconnect is blocked"
      )
    if (this.transport && this.ready && !this.lost) return
    if (!this.connecting)
      this.connecting = this.initialize().finally(() => {
        this.connecting = undefined
      })
    return this.connecting
  }
  private async initialize(): Promise<void> {
    await this.failure
    if (this.stopFailure) throw this.stopFailure
    if (this.transport) await this.transport.close()
    this.failure = undefined
    this.stopFailure = undefined
    this.unknownEvents = []
    this.lost = false
    const connection = await this.options.connect()
    if (this.closing || this.closed) {
      await connection.transport.close()
      throw new ExecutionError("unavailable", "Codex runtime closing")
    }
    if (connection.version !== protocol.PINNED_CODEX_VERSION) {
      await connection.transport.close()
      throw new ExecutionError(
        "unsupported",
        "Codex protocol version incompatible"
      )
    }
    const transport = connection.transport
    transport.onMessage((message) => {
      if (this.transport !== transport) return
      try {
        this.message(message)
      } catch {
        this.protocolFailure()
      }
    })
    transport.onDisconnect(() => {
      if (this.transport === transport) this.disconnected()
    })
    this.transport = transport
    try {
      protocol.initializeResult.parse(
        await transport.request("initialize", {
          clientInfo: { name: "factory", version: "0.0.1", title: "Factory" },
          capabilities: { experimentalApi: true, requestAttestation: false },
        } satisfies InitializeParams)
      )
      if (this.closing || this.closed) {
        await transport.close()
        throw new ExecutionError("unavailable", "Codex runtime closing")
      }
      transport.notify("initialized")
      this.ready = true
    } catch {
      await transport.close()
      this.transport = undefined
      if (this.closing)
        throw new ExecutionError("unavailable", "Codex runtime closing")
      throw new ExecutionError(
        "unsupported",
        "Codex initialization failed or protocol incompatible"
      )
    }
  }
  async probe(): Promise<RuntimeProbe> {
    const capabilities = {
      nativeDelegation: false,
      structuredQuestions: false,
      resume: false,
    }
    try {
      await this.connect()
      const account = protocol.accountResult.parse(
        await this.rpc().request("account/read", {
          refreshToken: false,
        } satisfies GetAccountParams)
      )
      if (!account.account || account.account.type !== "chatgpt")
        return {
          providerId: "codex",
          available: false,
          capabilities,
          reason:
            "Sign in to Codex with a managed ChatGPT account outside Factory",
        }
      return {
        providerId: "codex",
        available: true,
        capabilities: {
          nativeDelegation: true,
          structuredQuestions: true,
          resume: true,
        },
      }
    } catch (error) {
      return {
        providerId: "codex",
        available: false,
        capabilities,
        reason:
          error instanceof ExecutionError
            ? error.message
            : "Codex readiness protocol unsupported",
      }
    }
  }
  private rpc(): JsonRpcTransport {
    if (!this.transport)
      throw new ExecutionError("unavailable", "Codex connection unavailable")
    return this.transport
  }
  async listSkills(workspacePath: string): Promise<SkillReference[]> {
    if (!isAbsolute(workspacePath))
      throw new ExecutionError(
        "invalid-answer",
        "Workspace must be an absolute path"
      )
    await this.connect()
    const result = protocol.skillsResult.parse(
      await this.rpc().request("skills/list", {
        cwds: [workspacePath],
        forceReload: false,
      } satisfies SkillsListParams)
    )
    const entry = result.data.find((entry) => entry.cwd === workspacePath)
    if (!entry || entry.errors.length)
      throw new ExecutionError(
        "runtime",
        "Codex could not discover all workspace skills"
      )
    return entry.skills
      .filter((skill) => skill.enabled)
      .map(({ name, path }) => ({ name, path }))
  }
  async start(command: RuntimeStart): Promise<void> {
    if (this.starting)
      throw new ExecutionError("active-run", "Codex start already in progress")
    this.starting = command.runId
    try {
      await this.startOwned(command)
    } finally {
      this.starting = undefined
    }
  }
  private async startOwned(command: RuntimeStart): Promise<void> {
    await this.connect()
    if (this.interruptedRuns.has(command.runId))
      throw new ExecutionError("stale-request", "Start was stopped")
    if (
      this.runs.has(command.runId) ||
      [...this.runs.values()].some((run) => !run.terminal)
    )
      throw new ExecutionError(
        "active-run",
        "Codex already owns an active root"
      )
    if (!isAbsolute(command.workspacePath) || !command.task.trim())
      throw new ExecutionError(
        "invalid-answer",
        "Absolute workspace and nonempty task required"
      )
    if (command.skill) {
      const skills = await this.listSkills(command.workspacePath)
      if (
        !skills.some(
          (skill) =>
            skill.name === command.skill?.name &&
            skill.path === command.skill.path
        )
      )
        throw new ExecutionError(
          "unsupported",
          "Selected skill is not available in this workspace"
        )
    }
    if (this.interruptedRuns.has(command.runId))
      throw new ExecutionError("stale-request", "Start was stopped")
    let method = "thread/start"
    let params: { cwd: string; threadId?: string; excludeTurns?: boolean } = {
      cwd: command.workspacePath,
    }
    if (command.continuationOf) {
      const prior = this.runs.get(command.continuationOf.runId)
      const agent = prior?.agents.get(command.continuationOf.agentId)
      if (
        !prior?.terminal ||
        !agent ||
        prior.workspacePath !== command.workspacePath
      )
        throw new ExecutionError(
          "unsupported",
          "Conversation reference unavailable in this runtime session"
        )
      method = "thread/resume"
      params = {
        cwd: command.workspacePath,
        threadId: agent.threadId,
        excludeTurns: true,
      }
    }
    const agent: OwnedAgent = {
      id: command.rootAgentId,
      threadId: "",
      done: false,
    }
    const run: OwnedRun = {
      id: command.runId,
      workspacePath: command.workspacePath,
      rootId: agent.id,
      agents: new Map([[agent.id, agent]]),
      stopping: false,
      terminal: false,
      cleanupConfirmed: false,
    }
    this.runs.set(run.id, run)
    const result = protocol.threadResult.parse(
      await this.rpc().request(method, params)
    )
    if (run.stopping || run.terminal)
      throw new ExecutionError("stale-request", "Start was stopped")
    agent.threadId = result.thread.id
    this.threads.set(agent.threadId, { run, agent })
    this.emit(run, { type: "run-started" })
    const input: {
      type: string
      text?: string
      text_elements?: never[]
      name?: string
      path?: string
    }[] = [{ type: "text", text: command.task, text_elements: [] }]
    if (command.skill)
      input.push({
        type: "skill",
        name: command.skill.name,
        path: command.skill.path,
      })
    const turn = protocol.turnResult.parse(
      await this.rpc().request("turn/start", {
        threadId: agent.threadId,
        input,
      })
    )
    if (run.stopping || run.terminal) return
    agent.turnId = turn.turn.id
    this.replay()
  }
  async continue(command: RuntimeContinuation): Promise<void> {
    const run = this.active(command.runId)
    const agent = run.agents.get(command.agentId)
    if (!agent || agent.done || !agent.turnId)
      throw new ExecutionError("ownership", "Active agent unavailable")
    if ([...this.requests.values()].some((request) => request.agent === agent))
      throw new ExecutionError(
        "stale-request",
        "Resolve pending requests before continuation"
      )
    await this.rpc().request("turn/steer", {
      threadId: agent.threadId,
      expectedTurnId: agent.turnId,
      input: [{ type: "text", text: command.text, text_elements: [] }],
    } satisfies TurnSteerParams)
  }
  private active(runId: string): OwnedRun {
    const run = this.runs.get(runId)
    if (!run || run.terminal || run.stopping)
      throw new ExecutionError("stale-request", "Execution is no longer active")
    return run
  }
  async respond(response: RuntimeResponse): Promise<void> {
    const run = this.active(response.runId)
    const request = this.handles.get(response.requestId)
    if (
      !request ||
      request.run !== run ||
      request.agent.id !== response.agentId ||
      request.kind !== response.kind ||
      request.sent
    )
      throw new ExecutionError(
        "stale-request",
        "Exact pending request unavailable"
      )
    if (response.kind === "question") {
      const slot = request.slots.find(
        (slot) => slot.questionId === response.requestId
      )
      if (!slot || slot.submitted)
        throw new ExecutionError("stale-request", "Question already submitted")
      let value: string | undefined
      if (response.answer.optionId !== undefined)
        value = slot.options.get(response.answer.optionId)
      else if (slot.freeText && response.answer.text?.trim())
        value = response.answer.text
      if (value === undefined)
        throw new ExecutionError(
          "invalid-answer",
          "Supply one supported answer"
        )
      slot.value = value
      slot.submitted = true
    }
    const completion = new Promise<void>((resolve, reject) =>
      request.waiters.push({ resolve, reject })
    )
    if (
      response.kind === "permission" ||
      request.slots.every((slot) => slot.submitted)
    ) {
      request.sent = true
      request.timer = setTimeout(() => {
        this.invalidate(request)
        this.protocolFailure()
      }, this.options.responseTimeoutMs ?? 30_000)
      if (response.kind === "permission")
        this.rpc().respond(request.id, {
          decision: response.decision === "allow" ? "accept" : "decline",
        })
      else {
        const answers: Record<string, { answers: string[] }> = {}
        for (const slot of request.slots) {
          if (slot.value === undefined)
            throw new ExecutionError(
              "invalid-answer",
              "Incomplete question response"
            )
          Object.defineProperty(answers, slot.providerId, {
            value: { answers: [slot.value] },
            enumerable: true,
          })
        }
        this.rpc().respond(request.id, {
          answers,
        } satisfies ToolRequestUserInputResponse)
      }
    }
    return completion
  }
  private invalidate(request: PendingRequest): void {
    if (request.timer) clearTimeout(request.timer)
    this.requests.delete(request.id)
    for (const slot of request.slots) this.handles.delete(slot.questionId)
    for (const waiter of request.waiters)
      waiter.reject(
        new ExecutionError("stale-request", "Provider request invalidated")
      )
    request.waiters = []
  }
  private message(message: RpcMessage): void {
    if (message.method === "serverRequest/resolved") {
      const data = protocol.resolvedParams.parse(message.params)
      const request = this.requests.get(data.requestId)
      if (!request || request.agent.threadId !== data.threadId) return
      if (!request.sent) {
        this.invalidate(request)
        this.protocolFailure()
        return
      }
      if (request.timer) clearTimeout(request.timer)
      this.requests.delete(request.id)
      for (const slot of request.slots) this.handles.delete(slot.questionId)
      for (const waiter of request.waiters) waiter.resolve()
      return
    }
    const base = z.object({ threadId: z.string() }).safeParse(message.params)
    const owned = base.success
      ? this.threads.get(base.data.threadId)
      : undefined
    if (!owned) {
      if (message.id !== undefined) {
        this.rpc().reject(message.id)
        this.protocolFailure()
        return
      }
      if (
        base.success &&
        [
          "turn/started",
          "turn/completed",
          "item/started",
          "item/completed",
          "item/agentMessage/delta",
        ].includes(message.method)
      ) {
        if (this.unknownEvents.length >= 100) throw new Error("Event limit")
        this.unknownEvents.push(message)
      }
      return
    }
    const { run, agent } = owned
    const turnIdentity = z
      .object({ turnId: z.string() })
      .safeParse(message.params)
    if (
      turnIdentity.success &&
      agent.previousTurnIds?.has(turnIdentity.data.turnId)
    )
      return
    if (
      message.method === "turn/started" ||
      message.method === "turn/completed"
    ) {
      const data = protocol.turnParams.parse(message.params)
      if (agent.previousTurnIds?.has(data.turn.id)) return
    }
    if (run.stopping) {
      if (message.id !== undefined) {
        this.rpc().reject(message.id)
        return
      }
      if (message.method === "turn/completed") {
        const data = protocol.turnParams.parse(message.params)
        if (!agent.turnId || agent.turnId === data.turn.id) {
          if (data.turn.status === "inProgress")
            throw new Error("Nonterminal interruption outcome")
          agent.done = true
          agent.outcome = data.turn.status
        }
        return
      }
      if (message.method === "turn/started") {
        const data = protocol.turnParams.parse(message.params)
        if (agent.turnId !== data.turn.id) {
          agent.previousTurnIds = new Set([
            ...(agent.previousTurnIds ?? []),
            ...(agent.turnId ? [agent.turnId] : []),
          ])
          agent.outcome = undefined
        }
        agent.turnId = data.turn.id
        agent.done = false
        return
      }
      if (
        message.method === "item/started" ||
        message.method === "item/completed"
      ) {
        const data = protocol.itemParams.parse(message.params)
        if (data.item.type === "collabAgentToolCall")
          this.collaboration(run, agent, protocol.collabItem.parse(data.item))
      }
      return
    }
    if (!run.terminal && agent.done && message.method === "turn/started") {
      const data = protocol.turnParams.parse(message.params)
      if (data.turn.id === agent.turnId) return
      const renewed = this.renewAgent(run, agent)
      renewed.turnId = data.turn.id
      return
    }
    if (run.terminal || agent.done) {
      if (message.id !== undefined) this.rpc().reject(message.id)
      return
    }
    if (message.id !== undefined) {
      this.openRequest(message, run, agent)
      return
    }
    if (message.method === "turn/started") {
      const data = protocol.turnParams.parse(message.params)
      agent.turnId = data.turn.id
      return
    }
    if (message.method === "turn/completed") {
      const data = protocol.turnParams.parse(message.params)
      if (agent.turnId && agent.turnId !== data.turn.id) return
      if (data.turn.status === "inProgress")
        throw new Error("Nonterminal completion")
      if (
        [...this.requests.values()].some((request) => request.agent === agent)
      ) {
        this.protocolFailure()
        return
      }
      agent.outcome = data.turn.status
      if (agent.id === run.rootId) run.rootOutcome = data.turn.status
      this.finishAgents(run)
      return
    }
    if (message.method === "item/agentMessage/delta") {
      const data = protocol.deltaParams.parse(message.params)
      if (agent.turnId && agent.turnId !== data.turnId) return
      this.emit(run, {
        type: "message-delta",
        agentId: agent.id,
        messageId: this.messageId(agent, data.itemId),
        delta: data.delta,
      })
      return
    }
    if (
      message.method === "item/started" ||
      message.method === "item/completed"
    ) {
      const data = protocol.itemParams.parse(message.params)
      if (agent.turnId && agent.turnId !== data.turnId) return
      if (
        data.item.type === "agentMessage" &&
        message.method === "item/completed"
      ) {
        const item = protocol.messageItem.parse(data.item)
        const messageId = this.messageId(agent, item.id)
        const previous = this.completedMessages.get(messageId)
        if (previous !== undefined) {
          if (previous !== item.text)
            throw new Error("Conflicting completed message")
          return
        }
        this.completedMessages.set(messageId, item.text)
        this.emit(run, {
          type: "message-completed",
          message: {
            id: messageId,
            agentId: agent.id,
            text: item.text,
          },
        })
      } else if (data.item.type === "collabAgentToolCall")
        this.collaboration(run, agent, protocol.collabItem.parse(data.item))
      return
    }
    if (message.method === "error" || message.method === "thread/closed") {
      this.protocolFailure()
      return
    }
    // Ignore reasoning, raw/tool/account notifications; never persist private provider data.
  }
  private messageId(agent: OwnedAgent, itemId: string): string {
    const key = JSON.stringify([agent.id, itemId])
    let id = this.messageIds.get(key)
    if (!id) {
      id = this.createId()
      this.messageIds.set(key, id)
    }
    return id
  }
  private openRequest(
    message: RpcMessage,
    run: OwnedRun,
    agent: OwnedAgent
  ): void {
    if (message.id === undefined) return
    if (this.requests.has(message.id) || this.requests.size >= 100)
      throw new Error("Duplicate or excess requests")
    const request: PendingRequest = {
      id: message.id,
      run,
      agent,
      kind: "permission",
      slots: [],
      sent: false,
      waiters: [],
    }
    if (message.method === "item/tool/requestUserInput") {
      const data = protocol.questionParams.parse(message.params)
      if (
        data.turnId !== agent.turnId ||
        data.questions.some((question) => question.isSecret) ||
        new Set(data.questions.map((question) => question.id)).size !==
          data.questions.length
      ) {
        this.rpc().reject(message.id)
        this.protocolFailure()
        return
      }
      request.kind = "question"
      for (const question of data.questions) {
        const id = this.createId()
        const options = new Map<string, string>()
        for (const [index, option] of (question.options ?? []).entries())
          options.set(String(index), option.label)
        const freeText = question.isOther || options.size === 0
        request.slots.push({
          questionId: id,
          providerId: question.id,
          options,
          freeText,
          submitted: false,
        })
      }
      this.requests.set(request.id, request)
      for (const [index, slot] of request.slots.entries()) {
        const question = data.questions[index]
        if (!question) throw new Error("Question mismatch")
        this.handles.set(slot.questionId, request)
        this.emit(run, {
          type: "question-opened",
          question: {
            id: slot.questionId,
            agentId: agent.id,
            prompt: question.question,
            options: [...slot.options].map(([id, label]) => ({ id, label })),
            allowFreeText: slot.freeText,
            blocking: data.isBlocking,
            status: "pending",
          },
        })
      }
    } else if (message.method === "item/commandExecution/requestApproval") {
      const data = protocol.permissionParams.parse(message.params)
      const unsupportedPermission =
        data.grantRoot != null ||
        data.additionalPermissions != null ||
        data.networkApprovalContext != null ||
        data.kind === "writeStdin"
      const missingActionDetails = !data.command?.trim() || !data.cwd?.trim()
      if (unsupportedPermission || missingActionDetails) {
        this.rpc().reject(message.id)
        this.protocolFailure()
        return
      }
      if (
        data.turnId !== agent.turnId ||
        (data.availableDecisions &&
          (!data.availableDecisions.includes("accept") ||
            !data.availableDecisions.includes("decline")))
      ) {
        this.rpc().reject(message.id)
        this.protocolFailure()
        return
      }
      const id = this.createId()
      request.slots.push({
        questionId: id,
        providerId: "",
        options: new Map(),
        freeText: false,
        submitted: false,
      })
      this.requests.set(request.id, request)
      this.handles.set(id, request)
      this.emit(run, {
        type: "permission-requested",
        request: {
          id,
          agentId: agent.id,
          description: [
            "Codex requests permission to execute a command",
            data.command,
            data.cwd,
            data.reason,
          ]
            .filter(Boolean)
            .join("\n"),
          status: "pending",
        },
      })
    } else {
      this.rpc().reject(message.id)
      this.protocolFailure()
    }
  }
  private collaboration(
    run: OwnedRun,
    parent: OwnedAgent,
    item: z.infer<typeof protocol.collabItem>
  ): void {
    if (item.senderThreadId !== parent.threadId)
      throw new Error("Collaboration ownership mismatch")
    if (item.tool === "spawnAgent")
      for (const threadId of item.receiverThreadIds) {
        if (threadId === parent.threadId)
          throw new Error("Cyclic collaboration")
        const existing = this.threads.get(threadId)
        if (existing) {
          if (existing.run !== run || existing.agent.parentId !== parent.id)
            throw new Error("Foreign child")
          continue
        }
        const child: OwnedAgent = {
          id: this.createId(),
          threadId,
          parentId: parent.id,
          parentThreadId: parent.threadId,
          done: false,
        }
        run.agents.set(child.id, child)
        this.threads.set(threadId, { run, agent: child })
        if (!run.stopping)
          this.emit(run, {
            type: "agent-observed",
            agentId: child.id,
            parentId: parent.id,
          })
      }
    for (const [threadId, state] of Object.entries(item.agentsStates)) {
      const child = this.threads.get(threadId)
      if (!child || child.run !== run || child.agent.id === run.rootId) continue
      if (
        (state.status === "running" || state.status === "pendingInit") &&
        child.agent.done
      ) {
        this.renewAgent(run, child.agent)
        continue
      }
      if (
        [
          "completed",
          "interrupted",
          "errored",
          "shutdown",
          "notFound",
        ].includes(state.status)
      )
        child.agent.outcome =
          state.status === "completed"
            ? "completed"
            : state.status === "errored"
              ? "failed"
              : "interrupted"
    }
    this.replay()
    if (run.stopping) {
      for (const agent of run.agents.values())
        if (agent.outcome) agent.done = true
    } else this.finishAgents(run)
  }
  private renewAgent(run: OwnedRun, prior: OwnedAgent): OwnedAgent {
    if (!prior.parentThreadId)
      throw new Error("Only native child execution can renew")
    const parent = this.threads.get(prior.parentThreadId)
    if (!parent || parent.run !== run || (parent.agent.done && !run.stopping))
      throw new Error("Native parent execution unavailable")
    const child: OwnedAgent = {
      id: this.createId(),
      threadId: prior.threadId,
      parentId: parent.agent.id,
      parentThreadId: prior.parentThreadId,
      previousTurnIds: new Set([
        ...(prior.previousTurnIds ?? []),
        ...(prior.turnId ? [prior.turnId] : []),
      ]),
      done: false,
    }
    run.agents.set(child.id, child)
    this.threads.set(child.threadId, { run, agent: child })
    run.cleanupConfirmed = false
    if (!run.stopping)
      this.emit(run, {
        type: "agent-observed",
        agentId: child.id,
        parentId: parent.agent.id,
      })
    return child
  }
  private replay(): void {
    const events = this.unknownEvents
    this.unknownEvents = []
    for (const event of events) this.message(event)
  }
  private finishAgents(run: OwnedRun): void {
    let progressed = true
    while (progressed) {
      progressed = false
      for (const agent of run.agents.values()) {
        if (
          agent.id === run.rootId ||
          agent.done ||
          !agent.outcome ||
          [...run.agents.values()].some(
            (child) => child.parentId === agent.id && !child.done
          )
        )
          continue
        for (const request of this.requests.values())
          if (request.agent === agent) this.invalidate(request)
        agent.done = true
        this.emit(run, {
          type: "agent-finished",
          agentId: agent.id,
          outcome: agent.outcome,
        })
        progressed = true
      }
    }
    if (
      run.rootOutcome &&
      [...run.agents.values()].every(
        (agent) => agent.id === run.rootId || agent.done
      ) &&
      !run.terminal
    ) {
      run.terminal = true
      run.cleanupConfirmed = true
      const root = run.agents.get(run.rootId)
      if (root) root.done = true
      if (run.rootOutcome === "completed")
        this.emit(run, { type: "run-completed" })
      else
        this.emit(run, {
          type: run.rootOutcome === "failed" ? "run-failed" : "run-interrupted",
          reason: "Codex confirmed the terminal turn outcome",
        })
    }
  }
  async interrupt(runId: string): Promise<void> {
    this.interruptedRuns.add(runId)
    const run = this.runs.get(runId)
    if (!run || run.cleanupConfirmed) return
    run.stopping = true
    for (const request of this.requests.values())
      if (request.run === run) this.invalidate(request)
    const transport = this.transport
    if (!transport)
      throw new ExecutionError(
        "runtime",
        "Codex stop unconfirmed after connection loss"
      )
    const deadline = Date.now() + (this.options.stopTimeoutMs ?? 2_000)
    const requested = new Set<string>()
    while (Date.now() < deadline && !this.lost) {
      for (const agent of run.agents.values()) {
        if (agent.done || !agent.turnId) continue
        const key = agent.threadId + ":" + agent.turnId
        if (requested.has(key)) continue
        requested.add(key)
        // Request acknowledgement is not a resource-termination acknowledgement.
        void transport
          .request("turn/interrupt", {
            threadId: agent.threadId,
            turnId: agent.turnId,
          })
          .catch(() => {
            this.lost = true
          })
      }
      await new Promise<void>((resolve) => setTimeout(resolve, 20))
      if ([...run.agents.values()].every((agent) => agent.done)) break
    }
    const confirmed = [...run.agents.values()].every((agent) => agent.done)
    await transport.close()
    if (!confirmed)
      throw new ExecutionError(
        "runtime",
        "Native agent termination unconfirmed; owned app-server closed, execution remains cancelling"
      )
    run.terminal = true
    run.cleanupConfirmed = true
  }
  private protocolFailure(): void {
    if (this.failure) return
    this.failure = this.rpc()
      .close()
      .catch(() => {
        this.stopFailure = new ExecutionError(
          "runtime",
          "Codex cleanup unconfirmed"
        )
      })
  }
  private disconnected(): void {
    for (const request of this.requests.values()) this.invalidate(request)
    for (const run of this.runs.values())
      if (!run.terminal && !run.stopping) {
        run.terminal = true
        this.emit(run, {
          type: "provider-disconnected",
          reason: "Codex connection lost or unsupported protocol action",
        })
      }
    this.ready = false
    this.lost = true
  }
  async close(): Promise<void> {
    if (this.closed) return
    this.closing = true
    if (!this.closeInFlight)
      this.closeInFlight = this.closeOwned().finally(() => {
        this.closeInFlight = undefined
      })
    return this.closeInFlight
  }
  private async closeOwned(): Promise<void> {
    if (this.connecting) {
      try {
        await this.connecting
      } catch (error) {
        if (!(error instanceof ExecutionError) || error.code !== "unavailable")
          throw error
      }
    }
    let cleanupError: unknown
    for (const run of this.runs.values())
      if (!run.cleanupConfirmed) {
        try {
          await this.interrupt(run.id)
        } catch (error) {
          cleanupError ??= error
        }
      }
    await this.transport?.close()
    await this.failure
    if (this.stopFailure) throw this.stopFailure
    if (cleanupError) throw cleanupError
    this.closed = true
    this.listeners.clear()
  }
}
