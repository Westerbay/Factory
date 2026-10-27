import test from "node:test"
import assert from "node:assert/strict"
import { fixture, tick } from "./fixture.mjs"
for (const [name, payload] of [
  ["malformed JSON", "{not json}\n"],
  ["oversize frame", "x".repeat(100) + "\n"],
  ["unframed bound", "x".repeat(100)],
])
  test(name + " rejects pending requests and closes connection", async () => {
    const f = fixture({ handle: () => true, transport: { maxFrameBytes: 80 } })
    const pending = f.transport.request("wait", {})
    const rejection = assert.rejects(pending, /connection lost/)
    f.child.stdout.write(payload)
    await rejection
    await f.transport.close()
  })
test("process death and request timeout reject bounded pending work without stderr data", async () => {
  for (const death of [true, false]) {
    const f = fixture({ handle: () => true, timeoutMs: 20 })
    const pending = f.transport.request("wait", {})
    const rejection = assert.rejects(
      pending,
      (error) => !String(error).includes("SECRET")
    )
    f.child.stderr.write("SECRET".repeat(10000))
    if (death) f.child.emit("close", 1)
    await rejection
    await f.transport.close()
  }
})
test("unsent request resolved notification is not acceptance", async () => {
  const f = fixture()
  await f.runtime.start({
    runId: "run",
    rootAgentId: "root",
    workspacePath: "/workspace",
    task: "Task",
  })
  f.send({
    id: "rpc",
    method: "item/tool/requestUserInput",
    params: {
      threadId: "thread-root",
      turnId: "turn-root",
      itemId: "tool",
      autoResolutionMs: null,
      isBlocking: true,
      questions: [
        {
          id: "q",
          header: "Q",
          question: "Choose?",
          options: null,
          isOther: true,
          isSecret: false,
        },
      ],
    },
  })
  f.notify("serverRequest/resolved", {
    threadId: "thread-root",
    requestId: "rpc",
  })
  await tick()
  assert.ok(f.events.some((e) => e.type === "provider-disconnected"))
  await assert.rejects(f.runtime.close(), /termination unconfirmed/)
})
