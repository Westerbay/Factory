import test from "node:test"
import assert from "node:assert/strict"
import { ExecutionService } from "../dist/index.js"
import { ExecutionError } from "../../core/dist/index.js"
import { DeterministicRuntime, MemoryRunStore, deferred } from "./doubles.mjs"

function setup(store = new MemoryRunStore()) {
  const runtime = new DeterministicRuntime()
  let sequence = 0
  const service = new ExecutionService({
    runtime,
    store,
    createId: () => `local-${++sequence}`,
    now: () => "2026-10-26T10:00:00.000Z",
  })
  return { runtime, store, service }
}
const command = {
  workspacePath: "/fixture",
  task: "Ask and delegate",
  providerId: "test",
}
function question(agentId, id = "q1", blocking = true) {
  return {
    id,
    agentId,
    prompt: "Which approach?",
    options: [{ id: "a", label: "A" }],
    allowFreeText: true,
    blocking,
    status: "pending",
  }
}
async function observe(service, runtime, runId, body) {
  const event = runtime.event(runId, body)
  await service.acceptEvent(event)
  return event
}
const code = (expected) => (error) => error.code === expected

test("concurrent starts reserve one root even across coordinators and synchronous provider events", async () => {
  const store = new MemoryRunStore()
  const first = setup(store)
  const second = setup(store)
  const results = await Promise.allSettled([
    first.service.startRun(command),
    second.service.startRun(command),
  ])
  assert.equal(
    results.filter((result) => result.status === "fulfilled").length,
    1
  )
  assert.equal(
    results.find((result) => result.status === "rejected").reason.code,
    "active-run"
  )
  assert.equal(first.runtime.starts.length + second.runtime.starts.length, 1)
  assert.equal((await first.service.listRuns()).runs[0].status, "running")
})

test("child ownership, blocking scope, nonblocking questions and exact once-only answers", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await assert.rejects(
    observe(service, runtime, run.id, {
      type: "agent-observed",
      agentId: "foreign",
      parentId: "other-run-agent",
    }),
    code("ownership")
  )
  await observe(service, runtime, run.id, {
    type: "agent-observed",
    agentId: "child",
    parentId: run.rootAgentId,
  })
  await observe(service, runtime, run.id, {
    type: "agent-observed",
    agentId: "sibling",
    parentId: run.rootAgentId,
  })
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question("child"),
  })
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question("sibling", "q2", false),
  })
  const waiting = await service.getRun(run.id)
  assert.equal(waiting.status, "running")
  assert.equal(
    waiting.agents.find((agent) => agent.id === "child").status,
    "waiting-for-user"
  )
  assert.equal(
    waiting.agents.find((agent) => agent.id === "sibling").status,
    "running"
  )
  await assert.rejects(
    service.answerQuestion(run.id, "sibling", "q1", { text: "wrong owner" }),
    code("stale-request")
  )
  await assert.rejects(
    service.answerQuestion(run.id, "child", "q1", { optionId: "missing" }),
    code("invalid-answer")
  )
  const responses = await Promise.allSettled([
    service.answerQuestion(run.id, "child", "q1", { optionId: "a" }),
    service.answerQuestion(run.id, "child", "q1", { text: "duplicate" }),
  ])
  assert.equal(
    responses.filter((item) => item.status === "fulfilled").length,
    1
  )
  assert.equal(runtime.responses.length, 1)
  assert.equal(
    (await service.getRun(run.id)).agents.find((agent) => agent.id === "child")
      .status,
    "running"
  )
})

test("multiple blocking questions remain waiting until every owner request resolves", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question(run.rootAgentId),
  })
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question(run.rootAgentId, "q2"),
  })
  await service.answerQuestion(run.id, run.rootAgentId, "q1", { text: "first" })
  assert.equal((await service.getRun(run.id)).status, "waiting-for-user")
  await service.answerQuestion(run.id, run.rootAgentId, "q2", {
    text: "second",
  })
  assert.equal((await service.getRun(run.id)).status, "running")
})

test("permission requests stay separate and require an explicit decision", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "permission-requested",
    request: {
      id: "p1",
      agentId: run.rootAgentId,
      description: "Provider asks to write fixture",
      status: "pending",
    },
  })
  assert.equal(runtime.responses.length, 0)
  await assert.rejects(
    service.answerQuestion(run.id, run.rootAgentId, "p1", { text: "yes" }),
    code("stale-request")
  )
  await assert.rejects(
    service.continueRun(run.id, run.rootAgentId, "go"),
    code("invalid-transition")
  )
  await service.answerPermission(run.id, run.rootAgentId, "p1", "deny")
  assert.equal(runtime.responses[0].decision, "deny")
  await assert.rejects(
    service.answerPermission(run.id, run.rootAgentId, "p1", "allow"),
    code("stale-request")
  )
})

test("ordinary prose never opens a question; active steering and completed conversation reply are explicit", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "message-completed",
    message: { id: "m1", agentId: run.rootAgentId, text: "Shall I continue?" },
  })
  assert.equal((await service.getRun(run.id)).questions.length, 0)
  await service.continueRun(run.id, run.rootAgentId, "Please explain")
  assert.equal(runtime.continuations.length, 1)
  await observe(service, runtime, run.id, { type: "run-completed" })
  const next = await service.continueRun(
    run.id,
    run.rootAgentId,
    "Now continue"
  )
  assert.notEqual(next.id, run.id)
  assert.deepEqual(next.continuationOf, {
    runId: run.id,
    agentId: run.rootAgentId,
  })
  assert.equal((await service.getRun(run.id)).status, "completed")
  assert.deepEqual(runtime.starts[1].continuationOf, next.continuationOf)
  await assert.rejects(
    service.continueRun(run.id, run.rootAgentId, "Second simultaneous reply"),
    code("active-run")
  )
})

test("confirmed provider success is required; duplicate delivery is idempotent and history is paginated", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  const event = await observe(service, runtime, run.id, {
    type: "message-completed",
    message: { id: "m1", agentId: run.rootAgentId, text: "Done" },
  })
  await service.acceptEvent(event)
  assert.equal((await service.getRun(run.id)).status, "running")
  assert.equal(
    (await service.history(run.id)).events.filter(
      (item) => item.type === "message-completed"
    ).length,
    1
  )
  await observe(service, runtime, run.id, {
    type: "agent-observed",
    agentId: "child",
    parentId: run.rootAgentId,
  })
  await assert.rejects(
    observe(service, runtime, run.id, { type: "run-completed" }),
    code("invalid-transition")
  )
  await observe(service, runtime, run.id, {
    type: "agent-finished",
    agentId: "child",
    outcome: "completed",
  })
  await observe(service, runtime, run.id, { type: "run-completed" })
  await assert.rejects(
    observe(service, runtime, run.id, { type: "run-started" }),
    code("invalid-transition")
  )
  assert.equal((await service.getRun(run.id)).status, "completed")
  const page = await service.history(run.id, 0, 2)
  assert.equal(page.events.length, 2)
  assert.equal(page.nextSequence, 2)
  assert.equal(
    (await service.history(run.id, page.nextSequence)).events[0].id,
    event.id
  )
  await assert.rejects(service.listRuns(0, 201), code("invalid-answer"))
})

test("answer-vs-cancel invalidates in-flight requests and keeps cancelling until cleanup confirms", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question(run.rootAgentId),
  })
  const response = deferred()
  const responseStarted = deferred()
  const stopped = deferred()
  runtime.respondHook = () => {
    responseStarted.resolve()
    return response.promise
  }
  runtime.interruptHook = () => stopped.promise
  const answer = service.answerQuestion(run.id, run.rootAgentId, "q1", {
    text: "yes",
  })
  await responseStarted.promise
  const stop = service.cancelRun(run.id)
  await service.idle()
  assert.equal((await service.getRun(run.id)).status, "cancelling")
  assert.equal(
    (await service.getRun(run.id)).questions[0].status,
    "invalidated"
  )
  response.resolve()
  await assert.rejects(answer, code("invalid-transition"))
  stopped.resolve()
  assert.equal((await stop).status, "interrupted")
  await assert.rejects(
    service.answerQuestion(run.id, run.rootAgentId, "q1", { text: "late" }),
    code("invalid-transition")
  )
  assert.equal(runtime.responses.length, 1)
})

test("synchronous disconnect during respond never resolves a stale question", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question(run.rootAgentId),
  })
  runtime.respondHook = () => {
    runtime.emit(run.id, {
      type: "provider-disconnected",
      reason: "transport lost",
    })
  }
  await assert.rejects(
    service.answerQuestion(run.id, run.rootAgentId, "q1", { text: "yes" }),
    code("invalid-transition")
  )
  const lost = await service.getRun(run.id)
  assert.equal(lost.status, "interrupted")
  assert.equal(lost.questions[0].status, "invalidated")
})

test("synchronous stop event is safe; failed stop stays cancelling and can be explicitly retried", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  runtime.interruptHook = () => {
    throw new Error("busy")
  }
  await assert.rejects(service.cancelRun(run.id), code("runtime"))
  assert.equal((await service.getRun(run.id)).status, "cancelling")
  runtime.interruptHook = () => {
    runtime.emit(run.id, {
      type: "run-interrupted",
      reason: "all children stopped",
    })
  }
  assert.equal((await service.cancelRun(run.id)).status, "interrupted")
})

test("failed persistence never dispatches response or claims durability and triggers controlled stop", async () => {
  const { service, runtime, store } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question(run.rootAgentId),
  })
  const before = await store.get(run.id)
  store.failNextWrite = true
  await assert.rejects(
    service.answerQuestion(run.id, run.rootAgentId, "q1", { text: "yes" }),
    code("persistence")
  )
  await service.idle()
  assert.equal(runtime.responses.length, 0)
  assert.deepEqual(runtime.stops, [run.id])
  assert.equal(service.diagnostics().persistenceFailure.stopped, true)
  assert.deepEqual(await store.get(run.id), before)
  await assert.rejects(service.startRun(command), code("unavailable"))
  await assert.rejects(service.getRun(run.id), code("unavailable"))
})

test("storage loss on provider completion is visible; failed safety cleanup is not suppressed", async () => {
  const { service, runtime, store } = setup()
  const run = await service.startRun(command)
  store.failNextWrite = true
  runtime.interruptHook = () => {
    throw new Error("process still alive")
  }
  runtime.emit(run.id, { type: "run-completed" })
  await service.idle()
  assert.equal((await store.get(run.id)).status, "running")
  assert.equal(service.diagnostics().persistenceFailure.stopped, false)
  assert.ok(
    service
      .diagnostics()
      .errors.some((error) => error.message.includes("Safety stop failed"))
  )
})

test("startup failure is failed only after confirmed cleanup; unsupported resume does not dispatch", async () => {
  const { service, runtime } = setup()
  runtime.startHook = () => {
    throw new Error("startup unavailable")
  }
  await assert.rejects(service.startRun(command), code("runtime"))
  assert.equal((await service.listRuns()).runs[0].status, "failed")
  runtime.startHook = undefined
  const run = await service.startRun(command)
  runtime.capabilities.resume = false
  await assert.rejects(
    service.continueRun(run.id, run.rootAgentId, "hello"),
    code("unsupported")
  )
  assert.equal(runtime.continuations.length, 0)
})

test("provider completion arriving during a response waits for its explicit acceptance", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question(run.rootAgentId),
  })
  runtime.respondHook = () => {
    runtime.emit(run.id, { type: "run-completed" })
  }
  await service.answerQuestion(run.id, run.rootAgentId, "q1", { text: "yes" })
  await service.idle()
  assert.equal((await service.getRun(run.id)).status, "completed")
  assert.deepEqual(service.diagnostics().errors, [])
})

test("a terminal child invalidates its pending questions without touching siblings", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "agent-observed",
    agentId: "child",
    parentId: run.rootAgentId,
  })
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question("child"),
  })
  await observe(service, runtime, run.id, {
    type: "agent-finished",
    agentId: "child",
    outcome: "interrupted",
  })
  await assert.rejects(
    service.answerQuestion(run.id, "child", "q1", { text: "late" }),
    code("stale-request")
  )
  assert.equal((await service.getRun(run.id)).status, "running")
})

test("concurrent cancellation shares one in-flight stop and observes child cleanup", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "agent-observed",
    agentId: "child",
    parentId: run.rootAgentId,
  })
  const stopped = deferred()
  const stopping = deferred()
  runtime.interruptHook = () => {
    stopping.resolve()
    return stopped.promise
  }
  const first = service.cancelRun(run.id)
  const second = service.cancelRun(run.id)
  await stopping.promise
  await service.idle()
  await observe(service, runtime, run.id, {
    type: "agent-finished",
    agentId: "child",
    outcome: "interrupted",
  })
  assert.equal((await service.getRun(run.id)).status, "cancelling")
  assert.equal(runtime.stops.length, 1)
  stopped.resolve()
  await Promise.all([first, second])
})

test("persisted event replay ignores object key order but rejects changed nested content", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  const event = await observe(service, runtime, run.id, {
    type: "message-completed",
    message: {
      id: "stable-message",
      agentId: run.rootAgentId,
      text: "Original output",
    },
  })
  const reordered = {
    at: event.at,
    id: event.id,
    runId: event.runId,
    message: {
      text: event.message.text,
      agentId: event.message.agentId,
      id: event.message.id,
    },
    type: event.type,
  }
  await service.acceptEvent(reordered)
  const before = await service.history(run.id)
  await assert.rejects(
    service.acceptEvent({
      ...reordered,
      message: { ...reordered.message, text: "Changed output" },
    }),
    code("invalid-transition")
  )
  assert.deepEqual(await service.history(run.id), before)
  assert.equal(
    before.events.filter((item) => item.type === "message-completed").length,
    1
  )
})

test("structured request replay ignores option member order but preserves option sequence", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  const request = question(run.rootAgentId)
  request.options.push({ id: "b", label: "B" })
  const event = await observe(service, runtime, run.id, {
    type: "question-opened",
    question: request,
  })
  const reordered = {
    ...event,
    question: {
      ...event.question,
      options: event.question.options.map((option) => ({
        label: option.label,
        id: option.id,
      })),
    },
  }
  await service.acceptEvent(reordered)
  await assert.rejects(
    service.acceptEvent({
      ...reordered,
      question: {
        ...reordered.question,
        options: reordered.question.options.toReversed(),
      },
    }),
    code("invalid-transition")
  )
  assert.equal((await service.getRun(run.id)).questions.length, 1)
})

test("typed persistence failures on reads and writes still latch unavailable and stop owned work", async () => {
  for (const operation of ["get", "commit"]) {
    const { service, runtime, store } = setup()
    const run = await service.startRun(command)
    store[operation] = async () => {
      throw new ExecutionError("persistence", "Typed storage failure")
    }
    const attempt =
      operation === "get"
        ? service.getRun(run.id)
        : observe(service, runtime, run.id, {
            type: "message-completed",
            message: {
              id: "failed-write",
              agentId: run.rootAgentId,
              text: "Not durable",
            },
          })
    await assert.rejects(attempt, code("persistence"))
    await service.idle()
    assert.deepEqual(runtime.stops, [run.id])
    assert.equal(service.diagnostics().persistenceFailure.stopped, true)
    assert.ok(
      service.diagnostics().errors.some((error) => error.code === "persistence")
    )
    await assert.rejects(service.startRun(command), code("unavailable"))
  }
})

test("expected store conflicts do not stop healthy active work", async () => {
  const { service, runtime, store } = setup()
  const run = await service.startRun(command)
  store.commit = async () => {
    throw new ExecutionError("invalid-transition", "Version conflict")
  }
  await assert.rejects(
    observe(service, runtime, run.id, {
      type: "message-completed",
      message: {
        id: "conflict",
        agentId: run.rootAgentId,
        text: "Conflicting write",
      },
    }),
    code("invalid-transition")
  )
  await assert.rejects(service.startRun(command), code("active-run"))
  assert.deepEqual(runtime.stops, [])
  assert.equal(service.diagnostics().persistenceFailure, undefined)
  assert.equal((await service.getRun(run.id)).status, "running")
})

for (const kind of ["question", "permission"]) {
  test(`buffered provider success survives child ${kind} invalidation before response acknowledgement`, async () => {
    const { service, runtime } = setup()
    const run = await service.startRun(command)
    await observe(service, runtime, run.id, {
      type: "agent-observed",
      agentId: "child",
      parentId: run.rootAgentId,
    })
    if (kind === "question")
      await observe(service, runtime, run.id, {
        type: "question-opened",
        question: question("child"),
      })
    else
      await observe(service, runtime, run.id, {
        type: "permission-requested",
        request: {
          id: "p1",
          agentId: "child",
          description: "Provider permission",
          status: "pending",
        },
      })
    runtime.respondHook = () => {
      runtime.emit(run.id, { type: "run-completed" })
      runtime.emit(run.id, {
        type: "agent-finished",
        agentId: "child",
        outcome: "completed",
      })
    }
    const answer =
      kind === "question"
        ? service.answerQuestion(run.id, "child", "q1", {
            text: "Explicit reply",
          })
        : service.answerPermission(run.id, "child", "p1", "deny")
    await assert.rejects(answer, code("invalid-transition"))
    await service.idle()
    const completed = await service.getRun(run.id)
    assert.equal(completed.status, "completed")
    const requests =
      kind === "question" ? completed.questions : completed.permissions
    assert.equal(requests[0].status, "invalidated")
    const history = await service.history(run.id)
    assert.equal(
      history.events.filter((event) => event.type === "run-completed").length,
      1
    )
    assert.equal(
      history.events.some((event) => event.type === `${kind}-resolved`),
      false
    )
    assert.equal((await service.startRun(command)).status, "running")
  })
}

test("buffered success waits for both remaining children and unresolved requests", async () => {
  const { service, runtime } = setup()
  const run = await service.startRun(command)
  await observe(service, runtime, run.id, {
    type: "agent-observed",
    agentId: "child",
    parentId: run.rootAgentId,
  })
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question(run.rootAgentId),
  })
  await observe(service, runtime, run.id, {
    type: "question-opened",
    question: question(run.rootAgentId, "q2"),
  })
  runtime.respondHook = () => {
    runtime.emit(run.id, { type: "run-completed" })
  }
  await service.answerQuestion(run.id, run.rootAgentId, "q1", {
    text: "First explicit answer",
  })
  assert.equal((await service.getRun(run.id)).status, "waiting-for-user")
  await observe(service, runtime, run.id, {
    type: "agent-finished",
    agentId: "child",
    outcome: "completed",
  })
  assert.equal((await service.getRun(run.id)).status, "waiting-for-user")
  runtime.respondHook = undefined
  await service.answerQuestion(run.id, run.rootAgentId, "q2", {
    text: "Second explicit answer",
  })
  assert.equal((await service.getRun(run.id)).status, "completed")
  assert.deepEqual(service.diagnostics().errors, [])
})
