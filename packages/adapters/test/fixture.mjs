import { EventEmitter } from "node:events"
import { PassThrough } from "node:stream"
import { JsonRpcTransport } from "../dist/codex/transport.js"
import { CodexRuntime } from "../dist/index.js"
export function fixture(options = {}) {
  const child = new EventEmitter()
  child.stdout = new PassThrough()
  child.stderr = new PassThrough()
  child.stdin = new PassThrough()
  child.kill = () => {
    queueMicrotask(() => child.emit("close", 0))
    return true
  }
  const sent = []
  let buffer = ""
  const send = (message) => child.stdout.write(JSON.stringify(message) + "\n")
  child.stdin.on("data", (chunk) => {
    buffer += chunk
    let i
    while ((i = buffer.indexOf("\n")) >= 0) {
      const value = JSON.parse(buffer.slice(0, i))
      buffer = buffer.slice(i + 1)
      sent.push(value)
      queueMicrotask(() => {
        if (options.handle?.(value, send) === true) return
        if (value.method === "initialize")
          send({
            id: value.id,
            result: {
              userAgent: "codex/0.160.0",
              codexHome: "/private/not-persisted",
              platformFamily: "unix",
              platformOs: "linux",
            },
          })
        else if (value.method === "account/read")
          send({
            id: value.id,
            result: {
              account: options.noAccount
                ? null
                : { type: "chatgpt", email: "PRIVATE" },
              requiresOpenaiAuth: true,
            },
          })
        else if (value.method === "skills/list")
          send({
            id: value.id,
            result: {
              data: [
                {
                  cwd: "/workspace",
                  skills: [
                    {
                      name: "build",
                      path: "/workspace/.agents/skills/build/SKILL.md",
                      enabled: true,
                    },
                  ],
                  errors: [],
                },
              ],
            },
          })
        else if (
          value.method === "thread/start" ||
          value.method === "thread/resume"
        )
          send({
            id: value.id,
            result: { thread: { id: value.params.threadId ?? "thread-root" } },
          })
        else if (value.method === "turn/start") {
          send({
            method: "turn/started",
            params: {
              threadId: value.params.threadId,
              turn: { id: "turn-root", status: "inProgress" },
            },
          })
          send({
            id: value.id,
            result: { turn: { id: "turn-root", status: "inProgress" } },
          })
        } else if (value.method === "turn/interrupt") {
          send({ id: value.id, result: {} })
          if (!options.noInterruptConfirmation)
            send({
              method: "turn/completed",
              params: {
                threadId: value.params.threadId,
                turn: { id: value.params.turnId, status: "interrupted" },
              },
            })
        } else if (value.id && value.method) send({ id: value.id, result: {} })
        else if (
          value.id &&
          Object.hasOwn(value, "result") &&
          !options.manualAck
        )
          send({
            method: "serverRequest/resolved",
            params: {
              threadId: options.responseThread ?? "thread-root",
              requestId: value.id,
            },
          })
      })
    }
  })
  const transport = new JsonRpcTransport(child, {
    timeoutMs: options.timeoutMs ?? 1000,
    ...options.transport,
  })
  const runtime = new CodexRuntime({
    connect: async () => ({ transport, version: options.version ?? "0.160.0" }),
    responseTimeoutMs: 100,
    stopTimeoutMs: 80,
    ...options.runtime,
  })
  const events = []
  runtime.subscribe((event) => events.push(event))
  return {
    child,
    transport,
    runtime,
    events,
    sent,
    send,
    notify: (method, params) => send({ method, params }),
  }
}
export const start = {
  runId: "run",
  rootAgentId: "root",
  workspacePath: "/workspace",
  task: "Do the task",
}
export function requestQuestion(f, extra = {}) {
  f.send({
    id: "question-rpc",
    method: "item/tool/requestUserInput",
    params: {
      threadId: "thread-root",
      turnId: "turn-root",
      itemId: "tool",
      isBlocking: true,
      autoResolutionMs: null,
      questions: [
        {
          id: "a",
          header: "A",
          question: "Choose A",
          isOther: false,
          isSecret: false,
          options: [
            { label: "First", description: "First option" },
            { label: "Second", description: "Second option" },
          ],
        },
      ],
      ...extra,
    },
  })
}
export const tick = () => new Promise((resolve) => setImmediate(resolve))
