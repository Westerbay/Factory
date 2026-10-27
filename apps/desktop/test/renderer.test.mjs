import test from "node:test"
import assert from "node:assert/strict"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"
import { OperatorModel } from "../out/test-renderer/model.js"
import { ExecutionView } from "../out/test-renderer/ExecutionView.js"
const projection = (id, status = "completed") => ({
  id,
  workspacePath: "/fixture",
  providerId: "test",
  task: `Synthetic ${id}`,
  rootAgentId: `agent-${id}`,
  status,
  version: 1,
  createdAt: "2026-10-27T00:00:00.000Z",
  updatedAt: "2026-10-27T00:00:00.000Z",
  agents: [
    { id: `agent-${id}`, status: status === "cancelling" ? "running" : status },
  ],
  questions: [],
  permissions: [],
  messages: [],
})

test("the actual execution view allows explicit cancelling retry after the pending stop settles", () => {
  const run = projection("retry", "cancelling")
  const state = {
    host: { state: "ready", message: "ready" },
    run,
    runs: [run],
    selectedId: run.id,
    skills: [],
    history: [],
    deltas: {},
    busy: [],
  }
  const render = () =>
    renderToStaticMarkup(createElement(ExecutionView, { state }))
  assert.doesNotMatch(
    render(),
    /<button[^>]*disabled[^>]*>Stop execution<\/button>/
  )
  state.busy = ["stop"]
  assert.match(render(), /<button[^>]*disabled[^>]*>Stop execution<\/button>/)
  state.busy = []
  state.host.state = "unavailable"
  assert.doesNotMatch(
    render(),
    /<button[^>]*disabled[^>]*>Stop execution<\/button>/
  )
})

test("actual renderer history reaches records beyond four pages while keeping bounded memory and selection", async () => {
  const records = Array.from({ length: 251 }, (_, index) =>
    projection(`run-${index}`)
  )
  records[0] = projection("run-0", "running")
  globalThis.window = {
    factory: {
      command: async (command) => {
        if (command.type === "list-runs")
          return {
            type: "runs",
            runs: records.slice(command.offset, command.offset + command.limit),
            ...(command.offset + command.limit < records.length
              ? { nextOffset: command.offset + command.limit }
              : {}),
          }
        if (command.type === "get-run")
          return {
            type: "run",
            run: records.find((run) => run.id === command.runId),
          }
        if (command.type === "history") return { type: "history", events: [] }
        throw new Error("Unexpected synthetic command")
      },
    },
  }
  try {
    const model = new OperatorModel()
    await model.refresh()
    await model.selectRun("run-0")
    for (let index = 0; index < 5; index++) {
      await model.refresh(model.getSnapshot().nextOffset)
      assert.ok(model.getSnapshot().runs.length <= 50)
      assert.equal(model.getSnapshot().run.id, "run-0")
      assert.equal(model.getSnapshot().activeRunId, "run-0")
    }
    assert.ok(model.getSnapshot().runs.some((run) => run.id === "run-250"))
    await model.selectRun("run-250")
    assert.equal(model.getSnapshot().run.id, "run-250")
    await model.refresh(0)
    assert.ok(model.getSnapshot().runs.some((run) => run.id === "run-0"))
    assert.equal(model.getSnapshot().run.id, "run-250")
  } finally {
    delete globalThis.window
  }
})

async function offPageFixture() {
  const initial = projection("root", "running")
  let listener, pendingRead, nextRead
  const model = new OperatorModel()
  globalThis.window = {
    factory: {
      subscribe: (receive) => {
        listener = receive
        return () => {}
      },
      status: async () => ({
        state: "starting",
        message: "Synthetic test owns readiness",
      }),
      command: async (command) => {
        if (command.type === "list-runs")
          return {
            type: "runs",
            runs: command.offset ? [projection("older")] : [initial],
            nextOffset: command.offset ? undefined : 50,
          }
        if (command.type === "get-run") {
          if (nextRead) {
            nextRead = false
            return new Promise((resolve) => {
              pendingRead = resolve
            })
          }
          return {
            type: "run",
            run:
              command.runId === initial.id
                ? initial
                : projection(command.runId),
          }
        }
        if (command.type === "history") return { type: "history", events: [] }
        throw new Error("Unexpected synthetic command")
      },
    },
  }
  model.connect()
  await model.refresh()
  await model.selectRun(initial.id)
  await model.refresh(50)
  return {
    model,
    initial,
    emit: (run) => listener({ type: "run-state", run }),
    hold: () => {
      nextRead = true
    },
    resolve: (run) => pendingRead({ type: "run", run }),
  }
}

test("deferred off-page get cannot replace a newer selected terminal projection or reactivate it", async () => {
  try {
    const fixture = await offPageFixture()
    fixture.hold()
    const pending = fixture.model.selectRun("root")
    fixture.emit({ ...projection("root"), version: 2 })
    fixture.resolve(fixture.initial)
    await pending
    assert.equal(fixture.model.getSnapshot().run.version, 2)
    assert.equal(fixture.model.getSnapshot().run.status, "completed")
    assert.equal(fixture.model.getSnapshot().activeRunId, undefined)
  } finally {
    delete globalThis.window
  }
})

test("refreshing the off-page selection preserves its version while another root is active", async () => {
  try {
    const fixture = await offPageFixture()
    fixture.emit({ ...projection("root"), version: 2 })
    fixture.emit(projection("new-root", "running"))
    fixture.hold()
    const pending = fixture.model.selectRun("root")
    assert.equal(fixture.model.getSnapshot().run.version, 2)
    fixture.resolve(fixture.initial)
    await pending
    assert.equal(fixture.model.getSnapshot().run.version, 2)
    assert.equal(fixture.model.getSnapshot().activeRunId, "new-root")
    fixture.emit(fixture.initial)
    assert.equal(fixture.model.getSnapshot().run.version, 2)
    assert.equal(fixture.model.getSnapshot().activeRunId, "new-root")
    await fixture.model.refresh(0)
    assert.equal(
      fixture.model.getSnapshot().runs.find((run) => run.id === "root").version,
      2
    )
    assert.equal(fixture.model.getSnapshot().activeRunId, "new-root")
  } finally {
    delete globalThis.window
  }
})

test("an old stop result cannot relatch an evicted root after a newer root completed", async () => {
  try {
    const fixture = await offPageFixture()
    const ordinary = globalThis.window.factory.command
    let finish
    globalThis.window.factory.command = async (command) =>
      command.type === "cancel-run"
        ? new Promise((resolve) => {
            finish = resolve
          })
        : ordinary(command)
    const pending = fixture.model.stop()
    fixture.emit({ ...projection("root"), version: 2 })
    await fixture.model.selectRun("older")
    fixture.emit(projection("new-root", "running"))
    fixture.emit({ ...projection("new-root"), version: 2 })
    finish({ type: "run", run: fixture.initial })
    await pending
    assert.equal(fixture.model.getSnapshot().activeRunId, undefined)
    assert.equal(fixture.model.getSnapshot().run.id, "older")
  } finally {
    delete globalThis.window
  }
})
