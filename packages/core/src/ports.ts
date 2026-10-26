import type {
  Run,
  RunEvent,
  RuntimeEvent,
  SkillReference,
  QuestionAnswer,
  PermissionDecision,
  ContinuationReference,
} from "./model.js"
export interface RuntimeCapabilities {
  nativeDelegation: boolean
  structuredQuestions: boolean
  resume: boolean
}
export interface RuntimeProbe {
  providerId: string
  available: boolean
  capabilities: RuntimeCapabilities
  reason?: string
}
export interface RuntimeStart {
  runId: string
  rootAgentId: string
  workspacePath: string
  task: string
  skill?: SkillReference
  continuationOf?: ContinuationReference
}
export type RuntimeResponse =
  | {
      runId: string
      agentId: string
      kind: "question"
      requestId: string
      answer: QuestionAnswer
    }
  | {
      runId: string
      agentId: string
      kind: "permission"
      requestId: string
      decision: PermissionDecision
    }
export interface RuntimeContinuation {
  runId: string
  agentId: string
  text: string
}
export interface AgentRuntime {
  probe(): Promise<RuntimeProbe>
  listSkills(workspacePath: string): Promise<SkillReference[]>
  start(command: RuntimeStart): Promise<void>
  /** Resolves only after the exact provider request has accepted this response. Never auto-retry. */
  respond(response: RuntimeResponse): Promise<void>
  continue(command: RuntimeContinuation): Promise<void>
  /** Resolves only after all owned execution resources, including children, are stopped. */
  interrupt(runId: string): Promise<void>
  /** Listener delivery may happen synchronously during any runtime method. */
  subscribe(listener: (event: RuntimeEvent) => void): () => void
  close(): Promise<void>
}
export interface RunPage {
  runs: Run[]
  nextOffset?: number
}
export interface HistoryPage {
  events: RunEvent[]
  nextSequence?: number
}
/** Expected conflicts use ExecutionError codes active-run, invalid-transition, ownership or
 * not-found. Other failures, including typed persistence errors, indicate unavailable storage.
 */
export interface RunStore {
  /** Atomically creates snapshot + event; reject duplicate IDs and any existing active root. */
  create(run: Run, event: RunEvent): Promise<void>
  /** Atomically commits event + snapshot iff prior version matches; rejects duplicate event IDs. */
  commit(expectedVersion: number, run: Run, event: RunEvent): Promise<void>
  getActive(): Promise<Run | undefined>
  get(runId: string): Promise<Run | undefined>
  findEvent(runId: string, eventId: string): Promise<RunEvent | undefined>
  list(offset: number, limit: number): Promise<RunPage>
  /** Events are ordered by committed snapshot version, starting at sequence 1. */
  history(
    runId: string,
    afterSequence: number,
    limit: number
  ): Promise<HistoryPage>
}
