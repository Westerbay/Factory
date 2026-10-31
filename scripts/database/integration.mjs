import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { createServer } from "node:net"
import { createDatabase } from "../../packages/database/dist/index.js"
import { verifyApplicationLifecycle } from "../../packages/database/test/application.mjs"
import { verifyRelationalConstraints } from "../../packages/database/test/constraints.mjs"
import { reduceEvent } from "../../packages/core/dist/index.js"
import {
  root,
  docker,
  compose,
  inspectTarget,
  cleanupFixture,
} from "./owned.mjs"
const project = `factory-test-${randomUUID().replaceAll("-", "")}`
const port = await new Promise((resolve, reject) => {
  const server = createServer()
  server.on("error", reject)
  server.listen(0, "127.0.0.1", () => {
    const address = server.address()
    server.close(() => resolve(address.port))
  })
})
const target = {
  project,
  owner: randomUUID(),
  directory: root,
  database: "factory_test",
  username: "factory",
  password: "local-fixture-only",
  host: "127.0.0.1",
  port,
  container: `${project}-postgres-1`,
}
let created = false
let database, second
function run(id) {
  return {
    id,
    workspacePath: "/fixture",
    providerId: "fixture",
    task: "Synthetic persistence scenario",
    rootAgentId: `root-${id}`,
    status: "created",
    version: 1,
    createdAt: "2026-10-27T00:00:00.000Z",
    updatedAt: "2026-10-27T00:00:00.000Z",
    agents: [{ id: `root-${id}`, status: "created" }],
    questions: [],
    permissions: [],
    messages: [],
  }
}
function event(run, type, body = {}) {
  return {
    id: randomUUID(),
    runId: run.id,
    at: new Date().toISOString(),
    type,
    ...body,
  }
}
async function apply(store, current, type, body = {}) {
  const e = event(current, type, body),
    next = reduceEvent(current, e)
  await store.commit(current.version, next, e)
  return next
}
try {
  assert.equal(
    docker([
      "ps",
      "-a",
      "--filter",
      `name=^/${target.container}$`,
      "--format",
      "{{.ID}}",
    ]),
    ""
  )
  assert.equal(
    docker([
      "volume",
      "ls",
      "--filter",
      `name=^${project}_postgres_data$`,
      "--format",
      "{{.Name}}",
    ]),
    ""
  )
  created = true
  compose(target, ["up", "-d", "--wait", "postgres"])
  const proof = inspectTarget(target)
  database = createDatabase(proof)
  assert.deepEqual(await database.ready(), {
    available: false,
    reason: "uninitialized",
  })
  await database.initialize()
  await assert.rejects(database.initialize())
  assert.deepEqual(await database.ready(), { available: true })
  await verifyApplicationLifecycle(database.store)
  second = createDatabase(proof)
  const a = run("a"),
    b = run("b")
  const starts = await Promise.allSettled([
    database.store.create(a, event(a, "run-created")),
    second.store.create(b, event(b, "run-created")),
  ])
  assert.equal(starts.filter((s) => s.status === "fulfilled").length, 1)
  const rejected = starts.find((s) => s.status === "rejected")
  assert.equal(rejected.reason.code, "active-run")
  let current = await database.store.getActive()
  current = await apply(database.store, current, "run-started")
  current = await apply(database.store, current, "agent-observed", {
    agentId: "child",
    parentId: current.rootAgentId,
  })
  current = await apply(database.store, current, "question-opened", {
    question: {
      id: "q",
      agentId: "child",
      prompt: "Synthetic question",
      options: [],
      allowFreeText: true,
      blocking: true,
      status: "pending",
    },
  })
  current = await apply(database.store, current, "question-submitting", {
    questionId: "q",
  })
  current = await apply(database.store, current, "question-resolved", {
    questionId: "q",
    answer: { text: "Synthetic answer" },
  })
  const detached = await database.store.get(current.id)
  detached.questions[0].status = "invalidated"
  assert.equal(
    (await second.store.get(current.id)).questions[0].status,
    "resolved"
  )
  const e = event(current, "message-completed", {
      message: { id: "m", agentId: "child", text: "Synthetic completion" },
    }),
    next = reduceEvent(current, e)
  const race = await Promise.allSettled([
    database.store.commit(current.version, next, e),
    second.store.commit(current.version, next, e),
  ])
  assert.equal(race.filter((r) => r.status === "fulfilled").length, 1)
  assert.equal(
    race.find((r) => r.status === "rejected").reason.code,
    "invalid-transition"
  )
  current = next
  const duplicate = event(current, "continuation-requested", {
    agentId: current.rootAgentId,
    text: "Continue",
  })
  duplicate.id = e.id
  await assert.rejects(
    database.store.commit(
      current.version,
      reduceEvent(current, duplicate),
      duplicate
    ),
    { code: "invalid-transition" }
  )
  assert.equal((await database.store.get(current.id)).version, current.version)
  const badEvent = event(current, "message-completed", {
    message: { id: "bad", agentId: "foreign-agent", text: "Must rollback" },
  })
  const bad = {
    ...current,
    version: current.version + 1,
    messages: [...current.messages, badEvent.message],
  }
  await assert.rejects(database.store.commit(current.version, bad, badEvent), {
    code: "ownership",
  })
  assert.equal((await database.store.get(current.id)).version, current.version)
  assert.equal(
    await database.store.findEvent(current.id, badEvent.id),
    undefined
  )
  const page = await database.store.history(current.id, 0, 2)
  assert.equal(page.events.length, 2)
  assert.equal(page.nextSequence, 2)
  const rest = await database.store.history(current.id, page.nextSequence, 200)
  assert.equal(rest.events.length, current.version - 2)
  assert.equal(
    new Set([...page.events, ...rest.events].map((e) => e.id)).size,
    current.version
  )
  await second.close()
  await database.close()
  database = createDatabase(proof)
  assert.equal(
    (await database.store.get(current.id)).questions[0].status,
    "resolved"
  )
  current = await apply(database.store, current, "permission-requested", {
    request: {
      id: "permission",
      agentId: "child",
      description: "Synthetic permission",
      status: "pending",
    },
  })
  current = await apply(database.store, current, "question-opened", {
    question: {
      id: "q-pending",
      agentId: "child",
      prompt: "Pending on restart",
      options: [],
      allowFreeText: true,
      blocking: true,
      status: "pending",
    },
  })
  const finishConstraintFixture = await verifyRelationalConstraints(
    proof,
    current.id
  )
  try {
    await assert.rejects(database.recover(), { code: "persistence" })
    assert.equal(
      (await database.store.get(current.id)).version,
      current.version
    )
    assert.equal(
      (await database.store.history(current.id, current.version, 10)).events
        .length,
      0
    )
  } finally {
    await finishConstraintFixture()
  }
  assert.equal(await database.recover(), 1)
  assert.equal(await database.recover(), 0)
  const recovered = await database.store.get(current.id)
  assert.equal(recovered.status, "interrupted")
  assert.equal(
    recovered.questions.find((q) => q.id === "q-pending").status,
    "invalidated"
  )
  assert.equal(recovered.permissions[0].status, "invalidated")
  assert.equal(
    (await database.store.history(current.id, current.version, 10)).events[0]
      .type,
    "run-interrupted"
  )
  assert.throws(
    () =>
      reduceEvent(
        recovered,
        event(recovered, "question-submitting", { questionId: "q-pending" })
      ),
    { code: "invalid-transition" }
  )
  const fresh = run("fresh")
  await database.store.create(fresh, event(fresh, "run-created"))
  assert.equal((await database.store.list(0, 1)).nextOffset, 1)
  await database.close()
  assert.deepEqual(await database.ready(), {
    available: false,
    reason: "closed",
  })
  await assert.rejects(database.store.get("fresh"), { code: "persistence" })
  database = createDatabase(proof)
  compose(target, ["stop", "postgres"])
  assert.deepEqual(await database.ready(), {
    available: false,
    reason: "unavailable",
  })
  await assert.rejects(
    database.store.get("fresh"),
    (error) =>
      error.code === "persistence" &&
      !error.cause &&
      !error.message.includes("local-fixture-only")
  )
  // The same stopped-target preflight used by db:setup preserves identity and data.
  inspectTarget(target, { allowStopped: true })
  compose(target, ["up", "-d", "--wait", "postgres"])
  inspectTarget(target)
  assert.deepEqual(await database.ready(), { available: true })
  assert.equal((await database.store.get("fresh")).id, "fresh")
  await database.reset()
  assert.deepEqual(await database.ready(), { available: true })
  assert.equal(await database.store.get("fresh"), undefined)
  console.log(
    "PostgreSQL integration passed: initialization, reset, ownership rollback, concurrency, history, reconnect, recovery, unavailable and close"
  )
} finally {
  await second?.close()
  await database?.close()
  if (created) {
    cleanupFixture(target)
    assert.equal(
      docker([
        "ps",
        "-a",
        "--filter",
        `name=^/${target.container}$`,
        "--format",
        "{{.ID}}",
      ]),
      ""
    )
    assert.equal(
      docker([
        "volume",
        "ls",
        "--filter",
        `name=^${project}_postgres_data$`,
        "--format",
        "{{.Name}}",
      ]),
      ""
    )
    console.log("Exact owned fixture container, volume and network removed")
  }
}
