import { z } from "zod"

import { apiClient, ApiFailure, API_ORIGIN } from "../../lib/api/client"
import type { components } from "../../lib/api/schema"

export type ClientMessage = components["schemas"]["TerminalClientMessage"]
export type ServerMessage = components["schemas"]["TerminalServerMessage"]
export const terminalProtocol = "factory.terminal.v1"

const ticketSchema: z.ZodType<components["schemas"]["AttachmentResponse"]> =
  z.object({
    ticket: z.string().min(32).max(256),
    expiresAt: z.iso.datetime({ offset: true }),
  })
const errorCode = z.enum([
  "unauthorized",
  "unavailable",
  "conflict",
  "invalid_ticket",
  "invalid_input",
  "slow_client",
  "cleanup_uncertain",
])
export const serverMessageSchema: z.ZodType<ServerMessage> =
  z.discriminatedUnion("type", [
    z.object({ type: z.literal("ready") }),
    z.object({ type: z.literal("exit") }),
    z.object({ type: z.literal("detached"), code: errorCode }),
    z.object({ type: z.literal("error"), code: errorCode }),
  ])

export async function requestAttachment(
  takeOver: boolean,
  csrfToken: string,
  signal: AbortSignal
) {
  const { data, response } = await apiClient.POST("/api/terminal/attachments", {
    params: { header: { "X-CSRF-Token": csrfToken } },
    body: { takeOver },
    signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
  })
  if (!response.ok) throw new ApiFailure(response.status)
  const ticket = ticketSchema.parse(data)
  if (Date.parse(ticket.expiresAt) <= Date.now())
    throw new Error("Attachment expired")
  return ticket
}

export function openTerminalSocket() {
  const url = new URL("/api/terminal/socket", API_ORIGIN)
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:"
  return new WebSocket(url, terminalProtocol)
}

export function terminalProblem(
  code: components["schemas"]["TerminalErrorCode"]
): string {
  const messages = {
    unauthorized:
      "Your session is no longer active. Sign in again to reconnect.",
    unavailable:
      "The terminal is unavailable. Check that your environment is running, then reconnect.",
    conflict:
      "This terminal is open in another tab, or the environment changed. You can explicitly take over the terminal.",
    invalid_ticket:
      "The connection expired before it could open. Reconnect to try again.",
    invalid_input:
      "The terminal connection received an invalid message. Reconnect to try again.",
    slow_client:
      "The display could not keep up. Your command keeps running; reconnect to see its output.",
    cleanup_uncertain:
      "The previous terminal attachment could not be confirmed closed. Try an explicit takeover; contact your administrator if this continues.",
  }
  return messages[code]
}
