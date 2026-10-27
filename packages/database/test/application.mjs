import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { ExecutionService } from "../../application/dist/index.js"
import { DeterministicRuntime } from "../../application/test/doubles.mjs"

/** Exercise the unchanged application/RunStore seam against the real fixture database. */
export async function verifyApplicationLifecycle(store) {
  const runtime = new DeterministicRuntime()
  const service = new ExecutionService({
    runtime,
    store,
    createId: randomUUID,
    now: () => new Date().toISOString(),
  })
  runtime.startHook = async (command) => {
    const reserved = await store.get(command.runId)
    assert.equal(reserved.status, "created")
    assert.equal(reserved.agents[0].status, "created")
    runtime.emit(command.runId, { type: "run-started" })
  }
  try {
    const started = await service.startRun({
      workspacePath: "/fixture",
      providerId: "test",
      task: "Synthetic application persistence lifecycle",
    })
    assert.equal(started.status, "running")
    assert.equal(started.agents[0].status, "running")
    runtime.emit(started.id, {
      type: "question-opened",
      question: {
        id: "application-question",
        agentId: started.rootAgentId,
        prompt: "Synthetic persisted question",
        options: [],
        allowFreeText: true,
        blocking: true,
        status: "pending",
      },
    })
    await service.idle()
    assert.equal((await store.get(started.id)).status, "waiting-for-user")
    await service.answerQuestion(
      started.id,
      started.rootAgentId,
      "application-question",
      { text: "Synthetic explicit answer" }
    )
    assert.equal(runtime.responses.length, 1)
    assert.equal((await store.get(started.id)).questions[0].status, "resolved")
    const stopped = await service.cancelRun(started.id)
    assert.equal(stopped.status, "interrupted")
    assert.deepEqual(runtime.stops, [started.id])
    const history = await service.history(started.id)
    assert.deepEqual(
      history.events.map((event) => event.type),
      [
        "run-created",
        "run-started",
        "question-opened",
        "question-submitting",
        "question-resolved",
        "run-cancelling",
        "run-interrupted",
      ]
    )
    assert.equal(service.diagnostics().errors.length, 0)
  } finally {
    await service.close()
  }
}
