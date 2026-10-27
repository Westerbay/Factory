import test from "node:test"
import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import { DesktopHost } from "../out/test/runtime/host.js"
import { RuntimeClient } from "../out/test/main/runtime-client.js"
import { serveWorker } from "../out/test/runtime/worker-loop.js"
import { rendererAddress, isTrustedSender } from "../out/test/main/security.js"
import {
  DeterministicRuntime,
  MemoryRunStore,
  deferred,
} from "../../../packages/application/test/doubles.mjs"

async function setup({ available = true, recoverFailure = false } = {}) {
  const runtime = new DeterministicRuntime(),
    store = new MemoryRunStore(),
    wire = new EventEmitter(),
    incoming = new EventEmitter()
  if (!available)
    store.getActive = async () => {
      throw new Error("Tables unavailable before migrations")
    }
  let recoveries = 0,
    closed = false
  const client = new RuntimeClient(() => {})
  client.attach({
    on: wire.on.bind(wire),
    postMessage: (data) => incoming.emit("message", { data }),
  })
  const port = {
    on: incoming.on.bind(incoming),
    postMessage: (value) => wire.emit("message", value),
  }
  let host
  serveWorker(port, client.generation, async (send) => {
    host = new DesktopHost(
      {
        store,
        ready: async () => ({ available }),
        recover: async () => {
          recoveries++
          if (recoverFailure) throw new Error("private SQL password")
          return 0
        },
        close: async () => {
          closed = true
        },
      },
      runtime,
      (event) => send({ type: "event", generation: client.generation, event })
    )
    return host
  })
  await new Promise((resolve) => setTimeout(resolve, 10))
  return {
    runtime,
    store,
    client,
    wire,
    incoming,
    host,
    recoveries: () => recoveries,
    closed: () => closed,
  }
}
async function waitFor(predicate) {
  for (let n = 0; n < 100; n++) {
    if (await predicate()) return
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  assert.fail("condition did not settle")
}

test("built host/utility routing runs actual ExecutionService and keeps multi-question replies concurrent", async () => {
  const fixture = await setup(),
    { client, runtime, store } = fixture
  try {
    assert.equal(fixture.recoveries(), 1)
    assert.equal(
      (
        await client.registerWorkspace(
          { id: "selected", name: "fixture" },
          "/fixture"
        )
      ).type,
      "acknowledged"
    )
    assert.equal(
      (
        await client.command({
          type: "start-run",
          workspaceId: "forged",
          providerId: "test",
          task: "synthetic",
        })
      ).code,
      "ownership"
    )
    const started = await client.command({
      type: "start-run",
      workspaceId: "selected",
      providerId: "test",
      task: "synthetic",
    })
    assert.equal(started.type, "run")
    const run = started.run
    runtime.emit(run.id, {
      type: "agent-observed",
      agentId: "child",
      parentId: run.rootAgentId,
    })
    for (const id of ["first", "second"])
      runtime.emit(run.id, {
        type: "question-opened",
        question: {
          id,
          agentId: "child",
          prompt: "Choose explicitly",
          options: [],
          allowFreeText: true,
          blocking: true,
          status: "pending",
        },
      })
    await waitFor(async () => (await store.get(run.id)).questions.length === 2)
    const firstResponse = deferred()
    runtime.respondHook = async (response) => {
      if (response.requestId === "first") await firstResponse.promise
      else firstResponse.resolve()
    }
    let firstSettled = false
    const first = client
      .command({
        type: "answer-question",
        runId: run.id,
        agentId: "child",
        questionId: "first",
        answer: { text: "one" },
      })
      .then((result) => {
        firstSettled = true
        return result
      })
    await waitFor(
      async () => (await store.get(run.id)).questions[0].status === "submitting"
    )
    assert.equal(firstSettled, false)
    const second = await client.command({
      type: "answer-question",
      runId: run.id,
      agentId: "child",
      questionId: "second",
      answer: { text: "two" },
    })
    assert.equal(second.type, "acknowledged")
    assert.equal((await first).type, "acknowledged")
    assert.equal(
      (
        await client.command({
          type: "answer-question",
          runId: run.id,
          agentId: "child",
          questionId: "first",
          answer: { text: "duplicate" },
        })
      ).code,
      "stale-request"
    )
    assert.equal(
      (await client.command({ type: "cancel-run", runId: run.id })).run.status,
      "interrupted"
    )
    assert.equal(
      (
        await client.command({
          type: "history",
          runId: run.id,
          afterSequence: 0,
          limit: 100,
        })
      ).events.at(-1).type,
      "run-interrupted"
    )
  } finally {
    assert.equal((await client.close()).type, "acknowledged")
    assert.equal(fixture.closed(), true)
  }
})

test("missing persistence and failed recovery never admit commands or leak causes", async () => {
  for (const options of [{ available: false }, { recoverFailure: true }]) {
    const fixture = await setup(options)
    assert.equal(fixture.client.status().state, "unavailable")
    const result = await fixture.client.command({ type: "probe" })
    assert.equal(result.code, "unavailable")
    assert.doesNotMatch(JSON.stringify(result), /private SQL|password/)
    assert.equal(fixture.runtime.starts.length, 0)
    assert.equal((await fixture.client.close()).type, "acknowledged")
  }
})

test("worker death rejects pending replies and ignores old generations without claiming cleanup", async () => {
  const fixture = await setup(),
    { client, wire } = fixture
  const held = deferred()
  fixture.runtime.probe = () => held.promise
  const pending = client.command({ type: "probe" })
  wire.emit("message", {
    type: "status",
    generation: "stale",
    status: { state: "closed", message: "Old worker" },
  })
  assert.equal(client.status().state, "ready")
  wire.emit("exit", 1)
  const result = await pending
  assert.equal(result.code, "unavailable")
  assert.match(result.message, /cleanup is unconfirmed/)
  assert.equal(client.status().state, "unavailable")
  held.resolve({
    providerId: "test",
    available: true,
    capabilities: {
      nativeDelegation: true,
      structuredQuestions: true,
      resume: true,
    },
  })
  await fixture.host.close()
})

test("strict utility messages reject forged roots and stale workspace handles", async () => {
  const fixture = await setup()
  try {
    fixture.incoming.emit("message", {
      data: {
        type: "command",
        generation: fixture.client.generation,
        requestId: "forged",
        command: {
          type: "start-run",
          workspaceId: "x",
          workspacePath: "/private",
          providerId: "test",
          task: "bad",
        },
      },
    })
    await new Promise((resolve) => setTimeout(resolve, 5))
    assert.equal(fixture.runtime.starts.length, 0)
    assert.equal(
      (
        await fixture.client.command({
          type: "list-skills",
          workspaceId: "unknown",
        })
      ).code,
      "ownership"
    )
  } finally {
    await fixture.client.close()
  }
})

test("sender identity requires the exact owned window, top frame and document URL", () => {
  const frame = { url: "file:///app/index.html" },
    contents = { mainFrame: frame }
  const window = { isDestroyed: () => false, webContents: contents }
  assert.equal(
    isTrustedSender(
      { sender: contents, senderFrame: frame },
      window,
      frame.url
    ),
    true
  )
  assert.equal(
    isTrustedSender(
      { sender: contents, senderFrame: { url: frame.url } },
      window,
      frame.url
    ),
    false
  )
  assert.equal(
    isTrustedSender({ sender: {}, senderFrame: frame }, window, frame.url),
    false
  )
  assert.equal(
    isTrustedSender(
      { sender: contents, senderFrame: frame },
      window,
      frame.url + "?evil"
    ),
    false
  )
  assert.throws(() =>
    rendererAddress("http://localhost:5173/", false, "file:///app/index.html")
  )
  assert.throws(() =>
    rendererAddress(
      "http://127.0.0.1:5173/evil",
      false,
      "file:///app/index.html"
    )
  )
})

test("pending-capacity pressure keeps an independent stop lane available", async () => {
  const fixture = await setup()
  try {
    await fixture.client.registerWorkspace(
      { id: "workspace", name: "fixture" },
      "/fixture"
    )
    const started = await fixture.client.command({
      type: "start-run",
      workspaceId: "workspace",
      providerId: "test",
      task: "synthetic pressure",
    })
    const gate = deferred()
    fixture.runtime.probe = () => gate.promise
    const pending = Array.from({ length: 64 }, () =>
      fixture.client.command({ type: "probe" })
    )
    assert.equal(
      (await fixture.client.command({ type: "probe" })).code,
      "unavailable"
    )
    const stopped = await fixture.client.command({
      type: "cancel-run",
      runId: started.run.id,
    })
    assert.equal(stopped.run.status, "interrupted")
    gate.resolve({
      providerId: "test",
      available: true,
      capabilities: {
        nativeDelegation: true,
        structuredQuestions: true,
        resume: true,
      },
    })
    await Promise.all(pending)
  } finally {
    await fixture.client.close()
  }
})

test("a latched communication failure still permits explicit owned cleanup without reopening commands", async () => {
  const fixture = await setup()
  fixture.client.fail("Synthetic timeout; cleanup unconfirmed")
  assert.equal(
    (await fixture.client.command({ type: "probe" })).code,
    "unavailable"
  )
  assert.equal((await fixture.client.close()).type, "acknowledged")
  assert.equal(fixture.closed(), true)
  assert.equal(fixture.client.status().state, "closed")
})

test("bounded observation bursts preserve completed snapshots and signal temporary feed coalescing", async () => {
  const { ObservationRelay } =
    await import("../out/test/runtime/observation-relay.js")
  const delivered = [],
    relay = new ObservationRelay((event) => delivered.push(event))
  for (let i = 0; i < 1000; i++)
    relay.publish({
      type: "message-delta",
      durable: false,
      id: `delta-${i}`,
      runId: "run",
      at: "2026-10-27T00:00:00.000Z",
      agentId: "agent",
      messageId: "message",
      delta: "temporary",
    })
  assert.equal(delivered.length, 0)
  relay.publish({
    type: "run-state",
    run: { id: "run", messages: [{ id: "message", text: "durable" }] },
  })
  relay.flush()
  assert.ok(delivered.length <= 129)
  assert.equal(
    delivered.filter((event) => event.type === "message-delta").length,
    0
  )
  assert.equal(
    delivered.find((event) => event.type === "run-state").run.messages[0].text,
    "durable"
  )
  assert.match(
    delivered.find((event) => event.type === "diagnostic").message,
    /Refresh/
  )
  relay.close()
})

test("background persistence failure latches host status for fresh renderer snapshots", async () => {
  const fixture = await setup()
  try {
    await fixture.client.registerWorkspace(
      { id: "workspace", name: "fixture" },
      "/fixture"
    )
    fixture.store.failNextWrite = true
    const result = await fixture.client.command({
      type: "start-run",
      workspaceId: "workspace",
      providerId: "test",
      task: "synthetic storage failure",
    })
    assert.equal(result.code, "persistence")
    await waitFor(() => fixture.client.status().state === "unavailable")
    assert.equal(
      (await fixture.client.command({ type: "probe" })).code,
      "unavailable"
    )
    assert.equal(fixture.runtime.closed, true)
  } finally {
    await fixture.client.close()
  }
})

test("failed close preserves persistence and retries owned cleanup exactly once confirmed", async () => {
  const fixture = await setup()
  await fixture.client.registerWorkspace(
    { id: "workspace", name: "fixture" },
    "/fixture"
  )
  const started = await fixture.client.command({
    type: "start-run",
    workspaceId: "workspace",
    providerId: "test",
    task: "explicit close retry",
  })
  let attempts = 0
  fixture.runtime.interruptHook = async () => {
    if (++attempts === 1) throw new Error("cleanup not yet confirmed")
  }
  const first = await fixture.client.close()
  assert.equal(first.type, "error")
  assert.equal(fixture.closed(), false)
  assert.equal((await fixture.store.get(started.run.id)).status, "cancelling")
  assert.equal(
    (await fixture.client.command({ type: "probe" })).code,
    "unavailable"
  )
  assert.equal((await fixture.client.close()).type, "acknowledged")
  assert.equal(fixture.closed(), true)
  assert.equal((await fixture.store.get(started.run.id)).status, "interrupted")
  assert.equal(attempts, 2)
  assert.equal((await fixture.client.close()).type, "acknowledged")
  assert.equal(attempts, 2)
})

test("persistence failure plus immediate failed safety cleanup cannot mask fatal host status", async () => {
  const fixture = await setup()
  try {
    await fixture.client.registerWorkspace(
      { id: "workspace", name: "fixture" },
      "/fixture"
    )
    const started = await fixture.client.command({
      type: "start-run",
      workspaceId: "workspace",
      providerId: "test",
      task: "combined loss",
    })
    fixture.runtime.emit(started.run.id, {
      type: "provider-disconnected",
      reason: "Synthetic provider connection lost; native cleanup unconfirmed",
    })
    await waitFor(
      async () =>
        (await fixture.store.get(started.run.id)).status === "interrupted"
    )
    const recorded = []
    fixture.wire.on("message", (value) => recorded.push(value))
    const originalList = fixture.store.list.bind(fixture.store)
    fixture.store.list = async () => {
      throw new Error("private SQL database unavailable")
    }
    fixture.runtime.close = async () => {
      throw new Error("native cleanup unconfirmed")
    }
    assert.equal(
      (
        await fixture.client.command({
          type: "list-runs",
          offset: 0,
          limit: 50,
        })
      ).code,
      "persistence"
    )
    await waitFor(() => fixture.client.status().state === "unavailable")
    assert.ok(
      recorded.some(
        (message) =>
          message.type === "event" &&
          message.event.type === "diagnostic" &&
          message.event.code === "persistence"
      )
    )
    assert.ok(
      recorded.some(
        (message) =>
          message.type === "event" &&
          message.event.type === "diagnostic" &&
          message.event.code === "runtime"
      )
    )
    assert.equal(fixture.client.status().state, "unavailable")
    assert.match(
      fixture.client.status().message,
      /Persistence.*cleanup is unconfirmed/
    )
    assert.ok(
      recorded.some(
        (message) =>
          message.type === "event" &&
          message.event.type === "diagnostic" &&
          message.event.code === "runtime" &&
          message.event.message.includes("cleanup is unconfirmed")
      )
    )
    fixture.store.list = originalList
    fixture.runtime.close = async () => {}
  } finally {
    await fixture.client.close()
  }
})

test("a failed pool close remains unconfirmed without trusting a later adapter no-op", async () => {
  const store = new MemoryRunStore(),
    runtime = new DeterministicRuntime()
  let poolAttempts = 0,
    runtimeCloses = 0
  runtime.close = async () => {
    runtimeCloses++
  }
  const host = new DesktopHost(
    {
      store,
      ready: async () => ({ available: true }),
      recover: async () => 0,
      close: async () => {
        if (++poolAttempts === 1)
          throw new Error("Synthetic pool close failure")
      },
    },
    runtime,
    () => {}
  )
  await host.initialize()
  await assert.rejects(host.close(), /pool closure is unconfirmed/)
  await assert.rejects(host.close(), /pool closure is unconfirmed/)
  await assert.rejects(host.close(), /pool closure is unconfirmed/)
  assert.equal(poolAttempts, 1)
  assert.equal(runtimeCloses, 1)
  assert.equal((await host.command({ type: "probe" })).code, "unavailable")
})
