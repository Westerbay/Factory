import {
  ExecutionError,
  isTerminal,
  type Run,
  type RunEvent,
  type AgentExecution,
} from "./model.js"
export function requireAgent(run: Run, agentId: string): AgentExecution {
  const agent = run.agents.find((agent) => agent.id === agentId)
  if (!agent)
    throw new ExecutionError("ownership", "Agent does not belong to this run")
  return agent
}
export function requireActive(run: Run): void {
  if (isTerminal(run.status) || run.status === "cancelling")
    throw new ExecutionError(
      "invalid-transition",
      "Execution is stopped or stopping"
    )
}
function invalidate(run: Run): void {
  for (const question of run.questions)
    if (question.status !== "resolved") question.status = "invalidated"
  for (const request of run.permissions)
    if (request.status !== "resolved") request.status = "invalidated"
}
function refreshWaiting(run: Run, agentId: string): void {
  const agent = requireAgent(run, agentId)
  if (isTerminal(agent.status)) return
  const waiting = run.questions.some(
    (question) =>
      question.agentId === agentId &&
      question.blocking &&
      (question.status === "pending" || question.status === "submitting")
  )
  agent.status = waiting ? "waiting-for-user" : "running"
  if (agent.id === run.rootAgentId) run.status = agent.status
}
/** Pure projection. Callers handle duplicate delivery before committing the result. */
export function reduceEvent(current: Run, event: RunEvent): Run {
  if (current.id !== event.runId)
    throw new ExecutionError("ownership", "Event belongs to another run")
  const run: Run = {
    ...current,
    agents: current.agents.map((agent) => ({ ...agent })),
    questions: current.questions.map((question) => ({ ...question })),
    permissions: current.permissions.map((request) => ({ ...request })),
    messages: [...current.messages],
    version: current.version + 1,
    updatedAt: event.at,
  }
  if (isTerminal(run.status))
    throw new ExecutionError(
      "invalid-transition",
      "Terminal runs cannot receive new events"
    )
  if (
    run.status === "cancelling" &&
    !["run-interrupted", "provider-disconnected", "agent-finished"].includes(
      event.type
    )
  )
    throw new ExecutionError(
      "invalid-transition",
      "Only confirmed interruption can finish a cancelling run"
    )
  if (
    run.status === "created" &&
    ![
      "run-started",
      "run-cancelling",
      "run-failed",
      "provider-disconnected",
      "run-interrupted",
    ].includes(event.type)
  )
    throw new ExecutionError("invalid-transition", "Run has not started")
  switch (event.type) {
    case "run-created":
      throw new ExecutionError("invalid-transition", "Run is already created")
    case "run-started":
      if (run.status !== "created")
        throw new ExecutionError(
          "invalid-transition",
          "Run was already started"
        )
      run.status = "running"
      requireAgent(run, run.rootAgentId).status = "running"
      break
    case "agent-observed":
      if (run.status === "created")
        throw new ExecutionError("invalid-transition", "Run has not started")
      if (run.agents.some((agent) => agent.id === event.agentId))
        throw new ExecutionError("ownership", "Agent ID is already owned")
      if (isTerminal(requireAgent(run, event.parentId).status))
        throw new ExecutionError(
          "invalid-transition",
          "Parent agent is terminal"
        )
      run.agents.push({
        id: event.agentId,
        parentId: event.parentId,
        status: "running",
      })
      break
    case "agent-finished": {
      const agent = requireAgent(run, event.agentId)
      if (agent.id === run.rootAgentId || isTerminal(agent.status))
        throw new ExecutionError(
          "invalid-transition",
          "Root finishes through run outcome; child must be active"
        )
      if (
        run.agents.some(
          (child) => child.parentId === agent.id && !isTerminal(child.status)
        )
      )
        throw new ExecutionError(
          "invalid-transition",
          "Child agents are still active"
        )
      agent.status = event.outcome
      for (const question of run.questions)
        if (question.agentId === agent.id && question.status !== "resolved")
          question.status = "invalidated"
      for (const request of run.permissions)
        if (request.agentId === agent.id && request.status !== "resolved")
          request.status = "invalidated"
      break
    }
    case "message-completed":
      if (isTerminal(requireAgent(run, event.message.agentId).status))
        throw new ExecutionError(
          "invalid-transition",
          "Message agent is terminal"
        )
      if (run.messages.some((message) => message.id === event.message.id))
        throw new ExecutionError(
          "invalid-transition",
          "Message ID already completed"
        )
      run.messages.push({ ...event.message })
      break
    case "question-opened": {
      const question = event.question
      if (isTerminal(requireAgent(run, question.agentId).status))
        throw new ExecutionError(
          "invalid-transition",
          "Question agent is terminal"
        )
      if (
        question.status !== "pending" ||
        run.questions.some((item) => item.id === question.id)
      )
        throw new ExecutionError(
          "stale-request",
          "Question is not a fresh pending request"
        )
      if (
        !question.prompt.trim() ||
        (!question.allowFreeText && question.options.length === 0) ||
        new Set(question.options.map((option) => option.id)).size !==
          question.options.length
      )
        throw new ExecutionError(
          "invalid-answer",
          "Question has no valid response shape"
        )
      run.questions.push({
        ...question,
        options: question.options.map((option) => ({ ...option })),
      })
      refreshWaiting(run, question.agentId)
      break
    }
    case "question-submitting": {
      const question = run.questions.find(
        (question) => question.id === event.questionId
      )
      if (!question || question.status !== "pending")
        throw new ExecutionError(
          "stale-request",
          "Question is no longer pending"
        )
      question.status = "submitting"
      break
    }
    case "question-resolved": {
      const question = run.questions.find(
        (question) => question.id === event.questionId
      )
      if (!question || question.status !== "submitting")
        throw new ExecutionError(
          "stale-request",
          "Question response is no longer pending"
        )
      question.status = "resolved"
      refreshWaiting(run, question.agentId)
      break
    }
    case "permission-requested":
      if (isTerminal(requireAgent(run, event.request.agentId).status))
        throw new ExecutionError(
          "invalid-transition",
          "Permission agent is terminal"
        )
      if (
        event.request.status !== "pending" ||
        run.permissions.some((request) => request.id === event.request.id)
      )
        throw new ExecutionError(
          "stale-request",
          "Permission is not a fresh pending request"
        )
      run.permissions.push({ ...event.request })
      break
    case "permission-submitting": {
      const request = run.permissions.find(
        (request) => request.id === event.requestId
      )
      if (!request || request.status !== "pending")
        throw new ExecutionError(
          "stale-request",
          "Permission is no longer pending"
        )
      request.status = "submitting"
      break
    }
    case "permission-resolved": {
      const request = run.permissions.find(
        (request) => request.id === event.requestId
      )
      if (!request || request.status !== "submitting")
        throw new ExecutionError(
          "stale-request",
          "Permission response is no longer pending"
        )
      request.status = "resolved"
      break
    }
    case "continuation-requested": {
      const agent = requireAgent(run, event.agentId)
      const hasRequest =
        run.questions.some(
          (q) =>
            q.agentId === agent.id &&
            (q.status === "pending" || q.status === "submitting")
        ) ||
        run.permissions.some(
          (p) =>
            p.agentId === agent.id &&
            (p.status === "pending" || p.status === "submitting")
        )
      if (isTerminal(agent.status) || hasRequest)
        throw new ExecutionError(
          "invalid-transition",
          "Resolve outstanding requests before conversational continuation"
        )
      if (!event.text.trim())
        throw new ExecutionError(
          "invalid-answer",
          "Continuation cannot be empty"
        )
      break
    }
    case "run-cancelling":
      run.status = "cancelling"
      run.reason = event.reason
      invalidate(run)
      break
    case "run-completed":
      if (
        run.status === "created" ||
        run.questions.some(
          (q) => q.status === "pending" || q.status === "submitting"
        ) ||
        run.permissions.some(
          (p) => p.status === "pending" || p.status === "submitting"
        ) ||
        run.agents.some(
          (agent) => agent.id !== run.rootAgentId && !isTerminal(agent.status)
        )
      )
        throw new ExecutionError(
          "invalid-transition",
          "Provider success conflicts with active children or pending requests"
        )
      run.status = "completed"
      requireAgent(run, run.rootAgentId).status = "completed"
      break
    case "run-failed":
    case "run-interrupted":
    case "provider-disconnected":
      run.status = event.type === "run-failed" ? "failed" : "interrupted"
      run.reason = event.reason
      for (const agent of run.agents)
        if (!isTerminal(agent.status)) agent.status = run.status
      invalidate(run)
      break
  }
  return run
}
