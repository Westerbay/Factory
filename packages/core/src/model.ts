export type RunStatus =
  | "created"
  | "running"
  | "waiting-for-user"
  | "cancelling"
  | "completed"
  | "failed"
  | "interrupted"
export type AgentStatus =
  | "created"
  | "running"
  | "waiting-for-user"
  | "completed"
  | "failed"
  | "interrupted"
export type RequestStatus =
  | "pending"
  | "submitting"
  | "resolved"
  | "invalidated"
export interface SkillReference {
  name: string
  path: string
}
export interface AgentExecution {
  id: string
  parentId?: string
  status: AgentStatus
}
export interface QuestionOption {
  id: string
  label: string
}
export interface HumanQuestion {
  id: string
  agentId: string
  prompt: string
  options: QuestionOption[]
  allowFreeText: boolean
  blocking: boolean
  status: RequestStatus
}
export interface PermissionRequest {
  id: string
  agentId: string
  description: string
  status: RequestStatus
}
export interface CompletedMessage {
  id: string
  agentId: string
  text: string
}
export interface ContinuationReference {
  runId: string
  agentId: string
}
export interface Run {
  id: string
  workspacePath: string
  providerId: string
  skill?: SkillReference
  task: string
  continuationOf?: ContinuationReference
  rootAgentId: string
  status: RunStatus
  version: number
  createdAt: string
  updatedAt: string
  reason?: string
  agents: AgentExecution[]
  questions: HumanQuestion[]
  permissions: PermissionRequest[]
  messages: CompletedMessage[]
}
export interface StartRun {
  workspacePath: string
  providerId: string
  task: string
  skill?: SkillReference
}
export type QuestionAnswer =
  | { optionId: string; text?: never }
  | { text: string; optionId?: never }
export type PermissionDecision = "allow" | "deny"
export interface EventIdentity {
  id: string
  runId: string
  at: string
}
export type EventBody =
  | { type: "run-created" }
  | { type: "run-started" }
  | { type: "agent-observed"; agentId: string; parentId: string }
  | {
      type: "agent-finished"
      agentId: string
      outcome: "completed" | "failed" | "interrupted"
    }
  | { type: "message-completed"; message: CompletedMessage }
  | { type: "question-opened"; question: HumanQuestion }
  | { type: "question-submitting"; questionId: string }
  | { type: "question-resolved"; questionId: string; answer: QuestionAnswer }
  | { type: "permission-requested"; request: PermissionRequest }
  | { type: "permission-submitting"; requestId: string }
  | {
      type: "permission-resolved"
      requestId: string
      decision: PermissionDecision
    }
  | { type: "continuation-requested"; agentId: string; text: string }
  | { type: "run-cancelling"; reason: string }
  | { type: "run-completed" }
  | { type: "run-failed"; reason: string }
  | { type: "run-interrupted"; reason: string }
  | { type: "provider-disconnected"; reason: string }
export type RunEvent = EventIdentity & EventBody
export type RuntimeEvent = EventIdentity &
  (
    | { type: "run-started" }
    | { type: "agent-observed"; agentId: string; parentId: string }
    | {
        type: "agent-finished"
        agentId: string
        outcome: "completed" | "failed" | "interrupted"
      }
    | { type: "message-completed"; message: CompletedMessage }
    | {
        type: "message-delta"
        agentId: string
        messageId: string
        delta: string
      }
    | { type: "question-opened"; question: HumanQuestion }
    | { type: "permission-requested"; request: PermissionRequest }
    | { type: "run-completed" }
    | { type: "run-failed"; reason: string }
    | { type: "run-interrupted"; reason: string }
    | { type: "provider-disconnected"; reason: string }
  )
export class ExecutionError extends Error {
  constructor(
    public readonly code:
      | "active-run"
      | "not-found"
      | "ownership"
      | "invalid-transition"
      | "stale-request"
      | "invalid-answer"
      | "unsupported"
      | "persistence"
      | "runtime"
      | "unavailable",
    message: string,
    options?: ErrorOptions
  ) {
    super(message, options)
    this.name = "ExecutionError"
  }
}
export function isTerminal(status: RunStatus | AgentStatus): boolean {
  return (
    status === "completed" || status === "failed" || status === "interrupted"
  )
}
