import { z } from "zod"

import { apiClient, API_ORIGIN, ApiFailure } from "../../lib/api/client"
import type { components } from "../../lib/api/schema"

export type History = components["schemas"]["HistoryView"]
export type Conversation = components["schemas"]["ConversationView"]
export type Model = components["schemas"]["ModelView"]
export type Answer = components["schemas"]["AnswerView"]
const id = z.uuid()
const text = z.string()
const conversation = z.object({ id, createdAt: text })
const model = z.object({ id: text, name: text })
const run = z.object({
  id,
  conversationId: id,
  operationId: id,
  model: text,
  status: z.enum(["active", "completed", "failed", "interrupted", "uncertain"]),
})
const question = z.object({
  id,
  header: text,
  question: text,
  allowFreeText: z.boolean(),
  options: z.array(z.object({ label: text, description: text })),
})
const history = z.object({
  conversation,
  runsTruncated: z.boolean(),
  messagesTruncated: z.boolean(),
  requestsTruncated: z.boolean(),
  runs: z.array(run),
  messages: z.array(
    z.object({
      id,
      runId: id,
      role: z.enum(["user", "assistant"]),
      content: text,
      createdAt: text,
    })
  ),
  requests: z.array(
    z.object({
      id,
      runId: id,
      kind: z.literal("question"),
      state: z.enum(["pending", "claimed", "uncertain"]),
      questions: z.array(question),
    })
  ),
})
const event = z.object({
  kind: z.enum(["delta", "refresh"]),
  runId: id,
  text: z.string().nullish(),
})
const csrf = (token: string) => ({ "X-CSRF-Token": token })
function checked<T>(result: { data?: T; response: Response }): T {
  if (!result.response.ok) throw new ApiFailure(result.response.status)
  if (result.data === undefined) throw new Error("Missing response")
  return result.data
}
function check(response: Response) {
  if (!response.ok) throw new ApiFailure(response.status)
}
const conversationPage = z.object({
  items: z.array(conversation),
  nextCursor: id.nullable(),
})
export type ConversationPage = z.infer<typeof conversationPage>
export async function list(
  signal: AbortSignal,
  before?: string
): Promise<ConversationPage> {
  return conversationPage.parse(
    checked(
      await apiClient.GET("/api/conversations", {
        params: { query: before ? { before } : {} },
        signal,
      })
    )
  )
}
export async function models(signal: AbortSignal): Promise<Model[]> {
  return z
    .array(model)
    .parse(
      checked(await apiClient.GET("/api/conversations/models", { signal }))
    )
}
export async function create(
  token: string,
  signal: AbortSignal
): Promise<Conversation> {
  return conversation.parse(
    checked(
      await apiClient.POST("/api/conversations", {
        params: { header: csrf(token) },
        signal,
      })
    )
  )
}
export async function read(
  idValue: string,
  signal: AbortSignal
): Promise<History> {
  return history.parse(
    checked(
      await apiClient.GET("/api/conversations/{id}", {
        params: { path: { id: idValue } },
        signal,
      })
    )
  )
}
export async function start(
  idValue: string,
  body: components["schemas"]["StartBody"],
  token: string,
  signal: AbortSignal
) {
  return run.parse(
    checked(
      await apiClient.POST("/api/conversations/{id}/runs", {
        params: { path: { id: idValue }, header: csrf(token) },
        body,
        signal,
      })
    )
  )
}
export async function cancel(
  idValue: string,
  runId: string,
  token: string,
  signal: AbortSignal
) {
  const r = await apiClient.POST("/api/conversations/{id}/runs/{run}/cancel", {
    params: { path: { id: idValue, run: runId }, header: csrf(token) },
    signal,
  })
  check(r.response)
}
const recoverableEntry = z.object({
  conversationId: id,
  runId: id,
  launchSettled: z.boolean(),
})
export type Recoverable = z.infer<typeof recoverableEntry>
export async function listRecoverable(
  signal: AbortSignal
): Promise<Recoverable[]> {
  return z
    .array(recoverableEntry)
    .parse(
      checked(await apiClient.GET("/api/conversations/recoverable", { signal }))
    )
}
export async function recover(
  idValue: string,
  runId: string,
  token: string,
  signal: AbortSignal
) {
  const r = await apiClient.POST("/api/conversations/{id}/runs/{run}/recover", {
    params: { path: { id: idValue, run: runId }, header: csrf(token) },
    signal,
  })
  check(r.response)
}
/** Release an uncertain model probe only after a verified environment stop. */
export async function recoverModelProbe(token: string, signal: AbortSignal) {
  const result = await apiClient.POST(
    "/api/conversations/model-probe/recover",
    {
      params: { header: csrf(token) },
      signal,
    }
  )
  check(result.response)
}
export async function answer(
  idValue: string,
  runId: string,
  requestId: string,
  answers: Answer[],
  operationId: string,
  token: string,
  signal: AbortSignal
) {
  const r = await apiClient.POST(
    "/api/conversations/{id}/runs/{run}/questions/{request}",
    {
      params: {
        path: { id: idValue, run: runId, request: requestId },
        header: csrf(token),
      },
      body: { operationId, answers },
      signal,
    }
  )
  check(r.response)
}

/** Observe only: reconnect never resends input. A bounded parser rejects malformed streams. */
export async function stream(
  idValue: string,
  runId: string,
  signal: AbortSignal,
  onEvent: (value: z.infer<typeof event>) => void
) {
  const response = await fetch(
    `${API_ORIGIN}/api/conversations/${encodeURIComponent(idValue)}/runs/${encodeURIComponent(runId)}/events`,
    { credentials: "include", cache: "no-store", redirect: "error", signal }
  )
  check(response)
  if (!response.body) throw new Error("Missing event stream")
  const reader = response.body.pipeThrough(new TextDecoderStream()).getReader()
  let buffer = ""
  try {
    while (true) {
      const part = await reader.read()
      if (part.done) break
      buffer += part.value
      if (buffer.length > 256_000)
        throw new Error("Event stream exceeded its limit")
      let match: RegExpExecArray | null
      while ((match = /\r?\n\r?\n/.exec(buffer))) {
        const frame = buffer.slice(0, match.index).replaceAll("\r", "")
        buffer = buffer.slice(match.index + match[0].length)
        for (const line of frame.split("\n"))
          if (line.startsWith("data:")) {
            const value = event.parse(JSON.parse(line.slice(5).trim()))
            if (value.runId !== runId) throw new Error("Unexpected run event")
            onEvent(value)
          }
      }
    }
  } finally {
    reader.releaseLock()
  }
}
