import test from "node:test"
import assert from "node:assert/strict"
import { CodexRuntime } from "../dist/index.js"
import { ExecutionService } from "../../application/dist/index.js"
import { MemoryRunStore } from "../../application/test/doubles.mjs"
import { fixture, start, tick, requestQuestion } from "./fixture.mjs"
function application(runtime) {
  let n = 0
  return new ExecutionService({
    runtime,
    store: new MemoryRunStore(),
    createId: () => `application-${++n}`,
    now: () => new Date().toISOString(),
  })
}
function collab(
  f,
  tool,
  state = "running",
  thread = "child",
  sender = "thread-root",
  turn = "turn-root"
) {
  f.notify("item/completed", {
    threadId: sender,
    turnId: turn,
    item: {
      type: "collabAgentToolCall",
      id: `${tool}-${state}`,
      tool,
      senderThreadId: sender,
      receiverThreadIds: [thread],
      agentsStates: { [thread]: { status: state } },
    },
  })
}

test("application close and new starts preserve unconfirmed cleanup after spontaneous disconnect", async () => {
  const f = fixture()
  const app = application(f.runtime)
  const run = await app.startRun({
    providerId: "codex",
    workspacePath: "/workspace",
    task: "Task",
  })
  collab(f, "spawnAgent")
  f.notify("turn/started", {
    threadId: "child",
    turn: { id: "child-turn", status: "inProgress" },
  })
  f.child.emit("close", 1)
  await app.idle()
  assert.equal((await app.getRun(run.id)).status, "interrupted")
  await assert.rejects(f.runtime.interrupt(run.id), /unconfirmed/)
  await assert.rejects(app.close(), /unconfirmed/)
  assert.equal((await f.runtime.probe()).available, false)
  await assert.rejects(
    f.runtime.start({ ...start, runId: "another" }),
    /unconfirmed|closing|closed/
  )
})

test("each connection generation independently closes unsupported requests", async () => {
  const first = fixture()
  const second = fixture()
  const connections = [first, second]
  let next = 0
  const runtime = new CodexRuntime({
    connect: async () => ({
      transport: connections[next++].transport,
      version: "0.160.0",
    }),
  })
  for (const f of connections) {
    assert.equal((await runtime.probe()).available, true)
    f.send({ id: 500, method: "unsupported-action", params: {} })
    await tick()
    await assert.rejects(
      f.transport.request("still-alive", {}),
      /unavailable|closed/
    )
  }
  await runtime.close()
})

test("close awaits delayed connector and disposes result without initializing or becoming ready", async () => {
  const f = fixture()
  let release
  const runtime = new CodexRuntime({
    connect: () =>
      new Promise((resolve) => {
        release = () => resolve({ transport: f.transport, version: "0.160.0" })
      }),
  })
  const probing = runtime.probe()
  await tick()
  let finished = false
  const closing = runtime.close().then(() => {
    finished = true
  })
  await tick()
  assert.equal(finished, false)
  release()
  await closing
  assert.equal((await probing).available, false)
  assert.ok(
    !f.sent.some(
      (message) =>
        message.method === "initialize" || message.method === "account/read"
    )
  )
  await assert.rejects(
    f.transport.request("still-alive", {}),
    /unavailable|closed/
  )
})

test("native follow-up uses a fresh core execution identity and blocks root completion until renewed child settles", async () => {
  const f = fixture()
  const app = application(f.runtime)
  const run = await app.startRun({
    providerId: "codex",
    workspacePath: "/workspace",
    task: "Task",
  })
  collab(f, "spawnAgent")
  f.notify("turn/started", {
    threadId: "child",
    turn: { id: "child-one", status: "inProgress" },
  })
  f.notify("turn/completed", {
    threadId: "child",
    turn: { id: "child-one", status: "completed" },
  })
  await app.idle()
  const old = (await app.getRun(run.id)).agents.find((agent) => agent.parentId)
  collab(f, "followupTask")
  f.notify("turn/started", {
    threadId: "child",
    turn: { id: "child-two", status: "inProgress" },
  })
  requestQuestion(f, { threadId: "child", turnId: "child-two" })
  await app.idle()
  const renewed = (await app.getRun(run.id)).agents.find(
    (agent) => agent.parentId && agent.id !== old.id
  )
  assert.ok(renewed)
  assert.equal(renewed.parentId, run.rootAgentId)
  const q = (await app.getRun(run.id)).questions[0]
  assert.equal(q.agentId, renewed.id)
  // Fixture acknowledgement must identify the native child, not the root.
  const answer = app.answerQuestion(run.id, renewed.id, q.id, { optionId: "0" })
  await tick()
  f.notify("serverRequest/resolved", {
    threadId: "child",
    requestId: "question-rpc",
  })
  await answer
  f.notify("turn/completed", {
    threadId: "thread-root",
    turn: { id: "turn-root", status: "completed" },
  })
  await app.idle()
  assert.notEqual((await app.getRun(run.id)).status, "completed")
  f.notify("turn/completed", {
    threadId: "child",
    turn: { id: "child-two", status: "completed" },
  })
  await app.idle()
  assert.equal((await app.getRun(run.id)).status, "completed")
  assert.equal(app.diagnostics().errors.length, 0)
  await app.close()
})

test("file change approval without an inspectable patch is rejected through the application seam", async () => {
  const f = fixture()
  const app = application(f.runtime)
  const run = await app.startRun({
    providerId: "codex",
    workspacePath: "/workspace",
    task: "Task",
  })
  f.send({
    id: 73,
    method: "item/fileChange/requestApproval",
    params: {
      threadId: "thread-root",
      turnId: "turn-root",
      itemId: "patch",
      startedAtMs: 0,
    },
  })
  await tick()
  await app.idle()
  const state = await app.getRun(run.id)
  assert.equal(state.permissions.length, 0)
  assert.equal(state.status, "interrupted")
  assert.ok(f.sent.some((message) => message.id === 73 && message.error))
  await assert.rejects(app.close(), /unconfirmed/)
})

test("renewed nested execution keeps native ownership and stop interrupts its new turn", async () => {
  const f = fixture()
  const app = application(f.runtime)
  const run = await app.startRun({
    providerId: "codex",
    workspacePath: "/workspace",
    task: "Task",
  })
  collab(f, "spawnAgent")
  f.notify("turn/started", {
    threadId: "child",
    turn: { id: "parent-one", status: "inProgress" },
  })
  collab(f, "spawnAgent", "running", "nested", "child", "parent-one")
  f.notify("turn/started", {
    threadId: "nested",
    turn: { id: "nested-one", status: "inProgress" },
  })
  f.notify("turn/completed", {
    threadId: "nested",
    turn: { id: "nested-one", status: "completed" },
  })
  f.notify("turn/completed", {
    threadId: "child",
    turn: { id: "parent-one", status: "completed" },
  })
  await app.idle()
  collab(f, "resumeAgent")
  f.notify("turn/started", {
    threadId: "child",
    turn: { id: "parent-two", status: "inProgress" },
  })
  collab(f, "sendInput", "running", "nested", "child", "parent-two")
  // Delayed terminal delivery for the old turn must not settle renewed ownership.
  f.notify("turn/completed", {
    threadId: "nested",
    turn: { id: "nested-one", status: "completed" },
  })
  f.notify("turn/started", {
    threadId: "nested",
    turn: { id: "nested-two", status: "inProgress" },
  })
  await app.idle()
  const state = await app.getRun(run.id)
  const renewedParent = state.agents
    .filter((agent) => agent.parentId === run.rootAgentId)
    .at(-1)
  const renewedNested = state.agents
    .filter((agent) => agent.parentId === renewedParent.id)
    .at(-1)
  assert.ok(renewedNested)
  assert.equal(renewedNested.status, "running")
  assert.equal((await app.cancelRun(run.id)).status, "interrupted")
  assert.ok(
    f.sent.some(
      (message) =>
        message.method === "turn/interrupt" &&
        message.params.threadId === "nested" &&
        message.params.turnId === "nested-two"
    )
  )
  assert.equal(app.diagnostics().errors.length, 0)
  await app.close()
})

test("command consent requires nonempty command and working directory", async () => {
  for (const details of [
    {},
    { command: "echo hello" },
    { cwd: "/workspace" },
    { command: null, cwd: null },
    { command: " ", cwd: "/workspace" },
    { command: "echo hello", cwd: " " },
  ]) {
    const f = fixture()
    const app = application(f.runtime)
    const run = await app.startRun({
      providerId: "codex",
      workspacePath: "/workspace",
      task: "Task",
    })
    f.send({
      id: 74,
      method: "item/commandExecution/requestApproval",
      params: {
        threadId: "thread-root",
        turnId: "turn-root",
        itemId: "exec",
        ...details,
      },
    })
    await tick()
    await app.idle()
    const state = await app.getRun(run.id)
    assert.equal(state.permissions.length, 0)
    assert.equal(state.status, "interrupted")
    assert.ok(f.sent.some((message) => message.id === 74 && message.error))
    assert.ok(
      !f.sent.some(
        (message) => message.id === 74 && message.result?.decision === "accept"
      )
    )
    await assert.rejects(app.close(), /unconfirmed/)
  }
})

test("cancel waits for a renewed child whose next turn starts before the follow-up notification", async () => {
  let childRequested
  const childInterrupt = new Promise((resolve) => {
    childRequested = resolve
  })
  const f = fixture({
    runtime: { stopTimeoutMs: 500 },
    handle: (value, send) => {
      if (value.method !== "turn/interrupt") return false
      send({ id: value.id, result: {} })
      if (value.params.threadId === "thread-root") {
        send({
          method: "turn/started",
          params: {
            threadId: "child",
            turn: { id: "child-B", status: "inProgress" },
          },
        })
        send({
          method: "item/completed",
          params: {
            threadId: "thread-root",
            turnId: "turn-root",
            item: {
              type: "collabAgentToolCall",
              id: "followup-B",
              tool: "followupTask",
              senderThreadId: "thread-root",
              receiverThreadIds: ["child"],
              agentsStates: { child: { status: "running" } },
            },
          },
        })
        send({
          method: "turn/completed",
          params: {
            threadId: "thread-root",
            turn: { id: "turn-root", status: "interrupted" },
          },
        })
        send({
          method: "turn/completed",
          params: {
            threadId: "child",
            turn: { id: "child-A", status: "completed" },
          },
        })
      } else if (value.params.threadId === "child")
        childRequested(value.params.turnId)
      return true
    },
  })
  const app = application(f.runtime)
  const run = await app.startRun({
    providerId: "codex",
    workspacePath: "/workspace",
    task: "Task",
  })
  collab(f, "spawnAgent")
  f.notify("turn/started", {
    threadId: "child",
    turn: { id: "child-A", status: "inProgress" },
  })
  f.notify("turn/completed", {
    threadId: "child",
    turn: { id: "child-A", status: "completed" },
  })
  await app.idle()
  let settled = false
  const cancellation = app.cancelRun(run.id).then((result) => {
    settled = true
    return result
  })
  const requestedTurn = await Promise.race([
    childInterrupt,
    cancellation.then(() => undefined),
  ])
  assert.equal(
    requestedTurn,
    "child-B",
    "renewed work must be interrupted before cancel can succeed"
  )
  await tick()
  assert.equal(settled, false)
  assert.equal((await app.getRun(run.id)).status, "cancelling")
  f.notify("turn/completed", {
    threadId: "child",
    turn: { id: "child-B", status: "interrupted" },
  })
  assert.equal((await cancellation).status, "interrupted")
  assert.equal(app.diagnostics().errors.length, 0)
  await app.close()
})
