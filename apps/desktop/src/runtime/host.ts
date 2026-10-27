import { randomUUID } from "node:crypto"
import { ExecutionService } from "@workspace/application"
import { ExecutionError } from "@workspace/core"
import type { AgentRuntime, RunStore, SkillReference } from "@workspace/core"
import {
  desktopCommandSchema,
  desktopResultSchema,
  desktopEventSchema,
  type DesktopCommand,
  type DesktopEvent,
  type DesktopResult,
} from "@workspace/contracts"
import type { WorkspaceSelection } from "@workspace/contracts/desktop"

export interface HostDatabase {
  store: RunStore
  ready(): Promise<{ available: boolean; reason?: string }>
  recover(): Promise<number>
  close(): Promise<void>
}
const failureMessages = {
  "active-run":
    "An execution is already active. Stop it before starting another.",
  "not-found": "The requested execution was not found.",
  ownership: "This agent or request does not belong to the execution.",
  "invalid-transition":
    "This action is unavailable in the current execution state.",
  "stale-request":
    "This question or permission is no longer pending. Refresh the execution.",
  "invalid-answer":
    "Choose an offered answer or enter supported nonempty text.",
  unsupported:
    "The selected provider cannot perform this action. Check provider readiness.",
  persistence:
    "Persistence is unavailable. Stop work and restart after checking the owned database.",
  runtime:
    "The provider operation failed. Check readiness before retrying explicitly.",
  unavailable:
    "The runtime is unavailable. Restart after checking the database and provider.",
} satisfies Record<string, string>
export function hostError(error: unknown): DesktopResult {
  if (error instanceof ExecutionError)
    return {
      type: "error",
      code: error.code,
      message: failureMessages[error.code],
    }
  return { type: "error", code: "runtime", message: failureMessages.runtime }
}
export class PersistenceCloseError extends Error {
  constructor() {
    super(
      "Persistence pool closure is unconfirmed. This close cannot be safely retried. Check local database connections before quitting externally or restarting Factory."
    )
    this.name = "PersistenceCloseError"
  }
}
/** Utility-process owner: no filesystem roots or provider paths originate in renderer commands. */
export class DesktopHost {
  private service: ExecutionService
  private workspaces = new Map<
    string,
    { path: string; skills: Map<string, SkillReference> }
  >()
  private ready = false
  private initialized = false
  private closing?: Promise<void>
  private shutdownRequested = false
  private resourcesStopped = false
  private poolClose?: Promise<void>
  private persistenceUnavailable = false
  private unsubscribe: () => void
  private diagnosticTimer: ReturnType<typeof setInterval>
  private diagnosticCount = 0
  constructor(
    private database: HostDatabase,
    private runtime: AgentRuntime,
    publish: (event: DesktopEvent) => void,
    private closeFeed: () => void = () => {}
  ) {
    this.service = new ExecutionService({
      store: database.store,
      runtime,
      createId: randomUUID,
      now: () => new Date().toISOString(),
    })
    this.unsubscribe = this.service.subscribe((observation) => {
      if (observation.durable) {
        publish(
          desktopEventSchema.parse({ type: "run-state", run: observation.run })
        )
        publish(
          desktopEventSchema.parse({
            type: "run-event",
            durable: true,
            sequence: observation.run.version,
            event: observation.event,
          })
        )
      } else if (observation.event.type === "message-delta") {
        publish(
          desktopEventSchema.parse({ ...observation.event, durable: false })
        )
      }
    })
    this.diagnosticTimer = setInterval(() => {
      const diagnostics = this.service.diagnostics()
      if (diagnostics.persistenceFailure && !this.persistenceUnavailable) {
        this.persistenceUnavailable = true
        this.ready = false
        publish({
          type: "diagnostic",
          code: "persistence",
          message: diagnostics.persistenceFailure.stopped
            ? failureMessages.persistence
            : "Persistence is unavailable; owned runtime cleanup is unconfirmed. Check database and provider resources before restarting.",
        })
      }
      const errors = diagnostics.errors.slice(this.diagnosticCount)
      this.diagnosticCount = diagnostics.errors.length
      for (const failure of errors) {
        if (failure.code === "persistence" && diagnostics.persistenceFailure)
          continue
        publish({
          type: "diagnostic",
          code: failure.code,
          message: failureMessages[failure.code],
        })
      }
      if (
        errors.length &&
        diagnostics.persistenceFailure &&
        !diagnostics.persistenceFailure.stopped
      ) {
        publish({
          type: "diagnostic",
          code: "runtime",
          message:
            "Owned runtime cleanup is unconfirmed after persistence loss. Check local provider resources before restarting.",
        })
      }
    }, 250)
    this.diagnosticTimer.unref()
  }
  async initialize() {
    const readiness = await this.database.ready()
    if (!readiness.available)
      throw new ExecutionError(
        "persistence",
        "Owned database unavailable or pending migrations"
      )
    await this.database.recover()
    this.initialized = true
    this.ready = true
  }
  registerWorkspace(workspace: WorkspaceSelection, path: string) {
    if (this.workspaces.size >= 32)
      throw new ExecutionError(
        "unavailable",
        "Workspace selection limit reached; restart the app"
      )
    this.workspaces.set(workspace.id, { path, skills: new Map() })
  }
  private workspace(id: string) {
    const selection = this.workspaces.get(id)
    if (!selection)
      throw new ExecutionError(
        "ownership",
        "Select a workspace in the native dialog"
      )
    return selection
  }
  async command(input: DesktopCommand): Promise<DesktopResult> {
    try {
      const command = desktopCommandSchema.parse(input)
      if (!this.ready || this.shutdownRequested)
        throw new ExecutionError("unavailable", "Runtime is not ready")
      const result = await this.route(command)
      const diagnostics = this.service.diagnostics()
      if (diagnostics.persistenceFailure) this.ready = false
      return desktopResultSchema.parse(result)
    } catch (error) {
      return hostError(error)
    }
  }
  private async route(command: DesktopCommand): Promise<DesktopResult> {
    switch (command.type) {
      case "probe":
        return { type: "probe", probe: await this.runtime.probe() }
      case "list-skills": {
        const workspace = this.workspace(command.workspaceId)
        const references = await this.runtime.listSkills(workspace.path)
        workspace.skills.clear()
        const skills = references.slice(0, 500).map((reference) => {
          const id = randomUUID()
          workspace.skills.set(id, reference)
          return { id, reference }
        })
        return { type: "skills", skills }
      }
      case "start-run": {
        const workspace = this.workspace(command.workspaceId)
        const skill = command.skillId
          ? workspace.skills.get(command.skillId)
          : undefined
        if (command.skillId && !skill)
          throw new ExecutionError(
            "ownership",
            "Refresh the selected workspace skills"
          )
        return {
          type: "run",
          run: await this.service.startRun({
            workspacePath: workspace.path,
            providerId: command.providerId,
            task: command.task,
            ...(skill ? { skill } : {}),
          }),
        }
      }
      case "continue-run":
        return {
          type: "run",
          run: await this.service.continueRun(
            command.runId,
            command.agentId,
            command.text
          ),
        }
      case "answer-question":
        await this.service.answerQuestion(
          command.runId,
          command.agentId,
          command.questionId,
          command.answer
        )
        return { type: "acknowledged" }
      case "answer-permission":
        await this.service.answerPermission(
          command.runId,
          command.agentId,
          command.requestId,
          command.decision
        )
        return { type: "acknowledged" }
      case "cancel-run":
        return { type: "run", run: await this.service.cancelRun(command.runId) }
      case "get-run":
        return { type: "run", run: await this.service.getRun(command.runId) }
      case "list-runs":
        return {
          type: "runs",
          ...(await this.service.listRuns(command.offset, command.limit)),
        }
      case "history":
        return {
          type: "history",
          ...(await this.service.history(
            command.runId,
            command.afterSequence,
            command.limit
          )),
        }
    }
  }
  close(): Promise<void> {
    this.ready = false
    this.shutdownRequested = true
    if (!this.closing) {
      this.closing = this.closeOwned().catch((error: unknown) => {
        this.closing = undefined
        throw error
      })
    }
    return this.closing
  }
  private async closeOwned() {
    if (!this.resourcesStopped) {
      // Keep persistence available until confirmed cleanup has committed its terminal event.
      if (this.initialized) await this.service.close()
      else await this.runtime.close()
      this.resourcesStopped = true
      clearInterval(this.diagnosticTimer)
      this.unsubscribe()
      this.closeFeed()
    }
    // The database adapter marks itself closed before awaiting end(). A rejected pool
    // close therefore cannot be confirmed by a later no-op; keep that failure latched.
    this.poolClose ??= this.database.close().catch(() => {
      throw new PersistenceCloseError()
    })
    await this.poolClose
  }
}
