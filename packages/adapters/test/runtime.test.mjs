import test from "node:test"
import assert from "node:assert/strict"
import { ExecutionService } from "../../application/dist/index.js"
import { MemoryRunStore } from "../../application/test/doubles.mjs"
import { fixture, start, requestQuestion, tick } from "./fixture.mjs"

test("handshake, account readiness and workspace skill input keep protocol metadata private", async () => {
  const f = fixture()
  assert.equal((await f.runtime.probe()).available, true)
  const skills = await f.runtime.listSkills("/workspace")
  await f.runtime.start({ ...start, skill: skills[0] })
  assert.deepEqual(f.sent.find((v) => v.method === "account/read").params, {
    refreshToken: false,
  })
  assert.deepEqual(
    f.sent.find((v) => v.method === "turn/start").params.input[1],
    { type: "skill", ...skills[0] }
  )
  assert.ok(!JSON.stringify(f.events).includes("PRIVATE"))
  assert.ok(!JSON.stringify(f.events).includes("codexHome"))
  await f.runtime.close()
})
test("missing sign-in and incompatible executable never report readiness", async () => {
  for (const options of [{ noAccount: true }, { version: "0.159.0" }]) {
    const f = fixture(options)
    const result = await f.runtime.probe()
    assert.equal(result.available, false)
    assert.equal(result.capabilities.nativeDelegation, false)
    await f.runtime.close()
  }
})
test("actual application handles multiquestion concurrent responses only after exact provider acknowledgement", async () => {
  const f = fixture({ manualAck: true })
  let next = 0
  const store = new MemoryRunStore()
  const app = new ExecutionService({
    runtime: f.runtime,
    store,
    createId: () => `app-${++next}`,
    now: () => new Date().toISOString(),
  })
  const run = await app.startRun({
    providerId: "codex",
    workspacePath: "/workspace",
    task: "Task",
  })
  requestQuestion(f, {
    questions: ["a", "b"].map((id) => ({
      id,
      header: id,
      question: `Choose ${id}`,
      isOther: false,
      isSecret: false,
      options: [{ label: "Yes", description: "yes" }],
    })),
  })
  await app.idle()
  const snapshot = await app.getRun(run.id)
  assert.equal(snapshot.questions.length, 2)
  assert.equal(snapshot.status, "waiting-for-user")
  const first = app.answerQuestion(
    run.id,
    run.rootAgentId,
    snapshot.questions[0].id,
    { optionId: "0" }
  )
  await tick()
  assert.equal(f.sent.filter((v) => v.id === "question-rpc").length, 0)
  const second = app.answerQuestion(
    run.id,
    run.rootAgentId,
    snapshot.questions[1].id,
    { optionId: "0" }
  )
  await tick()
  assert.deepEqual(f.sent.find((v) => v.id === "question-rpc").result, {
    answers: { a: { answers: ["Yes"] }, b: { answers: ["Yes"] } },
  })
  assert.equal((await app.getRun(run.id)).questions[0].status, "submitting")
  f.notify("serverRequest/resolved", {
    threadId: "thread-root",
    requestId: "wrong",
  })
  await tick()
  assert.equal((await app.getRun(run.id)).questions[0].status, "submitting")
  f.notify("serverRequest/resolved", {
    threadId: "thread-root",
    requestId: "question-rpc",
  })
  await Promise.all([first, second])
  await app.idle()
  assert.ok(
    (await app.getRun(run.id)).questions.every((q) => q.status === "resolved")
  )
  await app.close()
})
test("stop invalidates pending/duplicate replies and rejects late acknowledgement", async () => {
  const f = fixture({ manualAck: true })
  await f.runtime.start(start)
  requestQuestion(f)
  const q = f.events.find((e) => e.type === "question-opened").question
  const pending = f.runtime.respond({
    runId: "run",
    agentId: "root",
    kind: "question",
    requestId: q.id,
    answer: { optionId: "0" },
  })
  const rejection = assert.rejects(pending, /invalidated/)
  await assert.rejects(
    f.runtime.respond({
      runId: "run",
      agentId: "root",
      kind: "question",
      requestId: q.id,
      answer: { optionId: "0" },
    }),
    /unavailable/
  )
  await f.runtime.interrupt("run")
  await rejection
  f.notify("serverRequest/resolved", {
    threadId: "thread-root",
    requestId: "question-rpc",
  })
  assert.ok(!f.events.some((e) => e.type === "run-completed"))
  await f.runtime.close()
})
test("permission remains separate and allows once without changing sandbox or approval policy", async () => {
  const f = fixture()
  await f.runtime.start(start)
  f.send({
    id: 77,
    method: "item/commandExecution/requestApproval",
    params: {
      threadId: "thread-root",
      turnId: "turn-root",
      itemId: "exec",
      command: "echo hello",
      cwd: "/workspace",
    },
  })
  const p = f.events.find((e) => e.type === "permission-requested").request
  await f.runtime.respond({
    runId: "run",
    agentId: "root",
    kind: "permission",
    requestId: p.id,
    decision: "deny",
  })
  assert.deepEqual(f.sent.find((v) => v.id === 77).result, {
    decision: "decline",
  })
  assert.ok(!JSON.stringify(f.sent).includes("approvalPolicy"))
  await f.runtime.close()
})
test("native children derive from spawn events, private reasoning is discarded and success waits for child completion", async () => {
  const f = fixture()
  await f.runtime.start(start)
  const item = {
    type: "collabAgentToolCall",
    id: "spawn",
    tool: "spawnAgent",
    senderThreadId: "thread-root",
    receiverThreadIds: ["child-thread"],
    agentsStates: { "child-thread": { status: "running" } },
  }
  f.notify("item/completed", {
    threadId: "thread-root",
    turnId: "turn-root",
    item,
  })
  const child = f.events.find((e) => e.type === "agent-observed")
  assert.equal(child.parentId, "root")
  f.notify("turn/started", {
    threadId: "child-thread",
    turn: { id: "child-turn", status: "inProgress" },
  })
  f.notify("item/reasoning/textDelta", {
    threadId: "child-thread",
    delta: "PRIVATE REASONING",
  })
  f.notify("item/agentMessage/delta", {
    threadId: "child-thread",
    turnId: "child-turn",
    itemId: "message",
    delta: "Hello",
  })
  f.notify("item/completed", {
    threadId: "child-thread",
    turnId: "child-turn",
    item: { type: "agentMessage", id: "message", text: "Hello world" },
  })
  f.notify("turn/completed", {
    threadId: "thread-root",
    turn: { id: "turn-root", status: "completed" },
  })
  assert.ok(!f.events.some((e) => e.type === "run-completed"))
  f.notify("turn/completed", {
    threadId: "child-thread",
    turn: { id: "child-turn", status: "completed" },
  })
  assert.equal(f.events.at(-1).type, "run-completed")
  assert.ok(!JSON.stringify(f.events).includes("PRIVATE REASONING"))
  await f.runtime.close()
})
test("active steer and terminal continuation map owned conversation references without reopening old runs", async () => {
  const f = fixture()
  await f.runtime.start(start)
  await f.runtime.continue({ runId: "run", agentId: "root", text: "More" })
  assert.equal(
    f.sent.find((v) => v.method === "turn/steer").params.expectedTurnId,
    "turn-root"
  )
  f.notify("turn/completed", {
    threadId: "thread-root",
    turn: { id: "turn-root", status: "completed" },
  })
  await f.runtime.start({
    ...start,
    runId: "run-next",
    rootAgentId: "root-next",
    continuationOf: { runId: "run", agentId: "root" },
  })
  assert.equal(
    f.sent.find((v) => v.method === "thread/resume").params.threadId,
    "thread-root"
  )
  await f.runtime.close()
})
test("unsupported actionable requests and secret questions fail visibly without answers", async () => {
  for (const secret of [false, true]) {
    const f = fixture()
    await f.runtime.start(start)
    if (secret)
      requestQuestion(f, {
        questions: [
          {
            id: "secret",
            header: "secret",
            question: "Token?",
            isOther: true,
            isSecret: true,
            options: null,
          },
        ],
      })
    else
      f.send({
        id: 1_000,
        method: "item/tool/call",
        params: { threadId: "thread-root", turnId: "turn-root" },
      })
    await tick()
    assert.ok(f.events.some((e) => e.type === "provider-disconnected"))
    assert.ok(!f.events.some((e) => e.type === "question-opened"))
    assert.ok(f.sent.some((v) => v.error?.code === -32601))
    await assert.rejects(f.runtime.close(), /termination unconfirmed/)
  }
})

test("interrupt RPC acknowledgement alone never confirms native cleanup", async () => {
  const f = fixture({ noInterruptConfirmation: true })
  await f.runtime.start(start)
  await assert.rejects(f.runtime.interrupt("run"), /termination unconfirmed/)
  assert.ok(
    !f.events.some(
      (event) =>
        event.type === "run-interrupted" || event.type === "run-completed"
    )
  )
  // Closing cannot upgrade absent native confirmation to a successful stop.
  await assert.rejects(f.runtime.close(), /termination unconfirmed/)
})

test("cancel racing thread creation never starts a model turn", async () => {
  let release
  const f = fixture({
    handle: (value, send) => {
      if (value.method === "thread/start") {
        release = () =>
          send({ id: value.id, result: { thread: { id: "late-thread" } } })
        return true
      }
      return false
    },
  })
  const pending = f.runtime.start(start)
  const rejected = assert.rejects(pending)
  while (!release) await tick()
  const stop = f.runtime.interrupt("run")
  release()
  await rejected
  await assert.rejects(stop, /unconfirmed/)
  assert.ok(!f.sent.some((value) => value.method === "turn/start"))
  await assert.rejects(f.runtime.close(), /termination unconfirmed/)
})

test("stop discovers and confirms a native child announced during interruption", async () => {
  let emitted = false
  const f = fixture({
    handle: (value, send) => {
      if (value.method !== "turn/interrupt") return false
      send({ id: value.id, result: {} })
      if (!emitted) {
        emitted = true
        send({
          method: "item/completed",
          params: {
            threadId: "thread-root",
            turnId: "turn-root",
            item: {
              type: "collabAgentToolCall",
              id: "late-spawn",
              tool: "spawnAgent",
              senderThreadId: "thread-root",
              receiverThreadIds: ["late-child"],
              agentsStates: { "late-child": { status: "running" } },
            },
          },
        })
        send({
          method: "turn/started",
          params: {
            threadId: "late-child",
            turn: { id: "late-turn", status: "inProgress" },
          },
        })
      }
      send({
        method: "turn/completed",
        params: {
          threadId: value.params.threadId,
          turn: { id: value.params.turnId, status: "interrupted" },
        },
      })
      return true
    },
  })
  await f.runtime.start(start)
  await f.runtime.interrupt("run")
  assert.deepEqual(
    f.sent
      .filter((v) => v.method === "turn/interrupt")
      .map((v) => v.params.threadId),
    ["thread-root", "late-child"]
  )
  assert.ok(!f.events.some((e) => e.type === "run-completed"))
  await f.runtime.close()
})

test("nonblocking child question leaves root running and completed messages use opaque wire-valid identities", async () => {
  const f = fixture()
  await f.runtime.start(start)
  requestQuestion(f, { isBlocking: false })
  const q = f.events.find((e) => e.type === "question-opened").question
  assert.equal(q.blocking, false)
  await f.runtime.respond({
    runId: "run",
    agentId: "root",
    kind: "question",
    requestId: q.id,
    answer: { optionId: "0" },
  })
  const payload = {
    threadId: "thread-root",
    turnId: "turn-root",
    item: {
      type: "agentMessage",
      id: "provider:id/with/private-format",
      text: "Public output",
    },
  }
  f.notify("item/completed", payload)
  f.notify("item/completed", payload)
  const completed = f.events.filter((e) => e.type === "message-completed")
  assert.equal(completed.length, 1)
  assert.match(completed[0].message.id, /^[A-Za-z0-9_-]{1,128}$/)
  assert.ok(!JSON.stringify(f.events).includes("private-format"))
  await f.runtime.close()
})

test("terminal clearing before response acknowledgement invalidates answer instead of claiming provider acceptance", async () => {
  const f = fixture({ manualAck: true })
  await f.runtime.start(start)
  requestQuestion(f)
  const question = f.events.find((e) => e.type === "question-opened").question
  const pending = f.runtime.respond({
    runId: "run",
    agentId: "root",
    kind: "question",
    requestId: question.id,
    answer: { optionId: "0" },
  })
  const rejection = assert.rejects(pending, /invalidated/)
  f.notify("turn/completed", {
    threadId: "thread-root",
    turn: { id: "turn-root", status: "completed" },
  })
  await rejection
  assert.ok(!f.events.some((e) => e.type === "run-completed"))
  assert.ok(f.events.some((e) => e.type === "provider-disconnected"))
  await assert.rejects(f.runtime.close(), /termination unconfirmed/)
})

test("cancel before asynchronous connection completes never starts a thread", async () => {
  const f = fixture()
  const starting = f.runtime.start(start)
  const rejected = assert.rejects(starting, /stopped/)
  await f.runtime.interrupt("run")
  await rejected
  assert.ok(
    !f.sent.some(
      (v) => v.method === "thread/start" || v.method === "turn/start"
    )
  )
  await f.runtime.close()
})
