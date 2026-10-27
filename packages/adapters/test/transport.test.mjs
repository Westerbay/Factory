import test from "node:test"
import assert from "node:assert/strict"
import { PassThrough } from "node:stream"
import { EventEmitter } from "node:events"
import { JsonRpcTransport } from "../dist/codex/transport.js"

test("transport correlates split and batched frames and never exposes provider errors", async () => {
  const child = new EventEmitter()
  child.stdout = new PassThrough()
  child.stderr = new PassThrough()
  child.stdin = new PassThrough()
  child.kill = () => {
    child.emit("close", 0)
    return true
  }
  const transport = new JsonRpcTransport(child, { timeoutMs: 100 })
  const first = transport.request("one", {})
  const second = transport.request("two", {})
  child.stdout.write('{"id":2,"result":{"ok":true}}\n{"id":1,')
  child.stdout.write('"error":{"code":-1,"message":"SECRET"}}\n')
  assert.deepEqual(await second, { ok: true })
  await assert.rejects(first, (error) => !String(error).includes("SECRET"))
  await transport.close()
})
