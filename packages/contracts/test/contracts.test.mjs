import test from "node:test"
import assert from "node:assert/strict"
import {
  desktopCommandSchema,
  desktopEventSchema,
  desktopResultSchema,
  runSchema,
  runEventSchema,
} from "../dist/index.js"
import { ExecutionService } from "../../application/dist/index.js"
import {
  MemoryRunStore,
  DeterministicRuntime,
} from "../../application/test/doubles.mjs"

test("commands reject privileged roots, arbitrary fields, missing consent, ambiguous answers and excessive pages", () => {
  assert.equal(
    desktopCommandSchema.safeParse({
      type: "start-run",
      workspacePath: "/etc",
      providerId: "test",
      task: "read",
    }).success,
    false
  )
  assert.equal(
    desktopCommandSchema.safeParse({
      type: "cancel-run",
      runId: "r1",
      shell: "rm",
    }).success,
    false
  )
  assert.equal(
    desktopCommandSchema.safeParse({
      type: "answer-permission",
      runId: "r1",
      agentId: "a1",
      requestId: "p1",
    }).success,
    false
  )
  for (const answer of [{}, { optionId: "a", text: "b" }, { text: "   " }])
    assert.equal(
      desktopCommandSchema.safeParse({
        type: "answer-question",
        runId: "r1",
        agentId: "a1",
        questionId: "q1",
        answer,
      }).success,
      false
    )
  assert.equal(
    desktopCommandSchema.safeParse({
      type: "history",
      runId: "r1",
      afterSequence: 0,
      limit: 201,
    }).success,
    false
  )
  assert.equal(
    desktopCommandSchema.safeParse({
      type: "execute-shell",
      command: "anything",
    }).success,
    false
  )
})

test("real application projections and material history cross the outbound schemas", async () => {
  let id = 0
  const runtime = new DeterministicRuntime()
  const store = new MemoryRunStore()
  const service = new ExecutionService({
    runtime,
    store,
    createId: () => `app-${++id}`,
    now: () => "2026-10-26T10:00:00.000Z",
  })
  const observations = []
  service.subscribe((observation) => observations.push(observation))
  const run = await service.startRun({
    workspacePath: "/fixture",
    providerId: "test",
    task: "hello",
  })
  const events = [
    { type: "agent-observed", agentId: "child", parentId: run.rootAgentId },
    {
      type: "question-opened",
      question: {
        id: "q1",
        agentId: "child",
        prompt: "Proceed?",
        options: [{ id: "yes", label: "Yes" }],
        allowFreeText: false,
        blocking: true,
        status: "pending",
      },
    },
    {
      type: "message-delta",
      agentId: run.rootAgentId,
      messageId: "m1",
      delta: "temporary",
    },
    {
      type: "message-completed",
      message: { agentId: run.rootAgentId, id: "m1", text: "Completed output" },
    },
  ]
  for (const event of events)
    await service.acceptEvent(runtime.event(run.id, event))
  await service.answerQuestion(run.id, "child", "q1", { optionId: "yes" })
  await service.acceptEvent(
    runtime.event(run.id, {
      type: "permission-requested",
      request: {
        id: "p1",
        agentId: "child",
        description: "Write fixture",
        status: "pending",
      },
    })
  )
  await service.answerPermission(run.id, "child", "p1", "deny")
  await service.continueRun(run.id, run.rootAgentId, "Explain")
  await service.acceptEvent(
    runtime.event(run.id, {
      type: "agent-finished",
      agentId: "child",
      outcome: "completed",
    })
  )
  await service.acceptEvent(runtime.event(run.id, { type: "run-completed" }))
  assert.equal(
    runSchema.parse(await service.getRun(run.id)).status,
    "completed"
  )
  for (const observation of observations) {
    if (observation.durable) {
      desktopEventSchema.parse({ type: "run-state", run: observation.run })
      desktopEventSchema.parse({
        type: "run-event",
        durable: true,
        sequence: observation.run.version,
        event: observation.event,
      })
    } else desktopEventSchema.parse({ ...observation.event, durable: false })
  }
  assert.ok(observations.some((observation) => !observation.durable))
  const history = await service.history(run.id)
  for (const event of history.events) runEventSchema.parse(event)
  assert.equal(
    history.events.some((event) => event.type === "message-delta"),
    false
  )
  desktopResultSchema.parse({ type: "history", ...history })
  desktopResultSchema.parse({ type: "runs", ...(await service.listRuns()) })
  desktopResultSchema.parse({ type: "probe", probe: await runtime.probe() })
  await service.close()
})

test("outbound events cannot label transient fragments durable or expose opaque implementation data", () => {
  assert.equal(
    desktopEventSchema.safeParse({
      type: "message-delta",
      durable: true,
      runId: "r1",
      agentId: "a1",
      messageId: "m1",
      id: "e1",
      at: "2026-10-26T10:00:00.000Z",
      delta: "partial",
    }).success,
    false
  )
  assert.equal(
    desktopResultSchema.safeParse({
      type: "error",
      code: "runtime",
      message: "failed",
      cause: { token: "private" },
    }).success,
    false
  )
})
