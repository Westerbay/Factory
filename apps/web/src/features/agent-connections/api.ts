import { z } from "zod"

import { apiClient, ApiFailure } from "../../lib/api/client"
import type { components } from "../../lib/api/schema"

export type Connection = components["schemas"]["AgentConnectionResponse"]
export type LoginChallenge = components["schemas"]["AgentLoginChallenge"]
export function canDisconnectConnection(
  connection: Connection | null
): boolean {
  return (
    connection?.status === "connected" ||
    connection?.status === "unavailable" ||
    connection?.status === "error"
  )
}
export type ConnectionAction =
  | { type: "login" }
  | { type: "cancel"; operationId: string }
  | { type: "logout" }

export function isOfficialVerificationUrl(value: string): boolean {
  try {
    const url = new URL(value)
    return (
      url.origin === "https://auth.openai.com" &&
      url.pathname === "/codex/device" &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash
    )
  } catch {
    return false
  }
}

export const safeText = (max: number) =>
  z
    .string()
    .min(1)
    .max(max)
    .refine(
      (value) =>
        [...value].every((character) => {
          const code = character.charCodeAt(0)
          return code > 31 && (code < 127 || code > 159)
        }),
      "Invalid account text"
    )
export const connectionSchema: z.ZodType<Connection> = z
  .object({
    provider: z.literal("codex"),
    status: z.enum([
      "unavailable",
      "disconnected",
      "connecting",
      "connected",
      "error",
    ]),
    account: z
      .object({ email: safeText(320).nullish(), plan: safeText(64) })
      .nullish(),
    challenge: z
      .object({
        operationId: z.uuid(),
        verificationUrl: z.string().max(2048).refine(isOfficialVerificationUrl),
        userCode: z
          .string()
          .min(1)
          .max(64)
          .regex(/^[a-zA-Z0-9-]+$/),
        expiresAt: z.iso.datetime({ offset: true }),
      })
      .nullish(),
    errorCode: z.string().max(100).nullish(),
    observedAt: z.iso.datetime({ offset: true }).nullish(),
  })
  .refine(
    (value) =>
      (value.status === "connecting") === Boolean(value.challenge) &&
      (value.status === "connected") === Boolean(value.account),
    "Inconsistent account state"
  )

export class ConnectionFailure extends ApiFailure {
  constructor(
    status: number,
    public readonly code?: string
  ) {
    super(status)
  }
}

const problems: Record<string, string> = {
  unavailable: "Codex is unavailable. Check again in a moment.",
  unsupported_method:
    "This environment uses an API key or another unsupported method. Connect your personal ChatGPT/Codex account instead.",
  login_failed:
    "Codex did not confirm sign-in. Check the status before trying again.",
  invalid_provider_response:
    "The Codex response could not be verified. Check the status before trying again.",
  cancelled: "Sign-in was cancelled.",
  expired:
    "The sign-in code has expired. Check the status before starting again.",
  timeout:
    "The operation timed out. Its result is unknown; check the status again.",
  cleanup_uncertain:
    "Sign-in may still be closing. Check again before retrying; recovery can take up to 10 minutes.",
}
export function connectionProblem(code?: string | null): string | undefined {
  return code && Object.hasOwn(problems, code) ? problems[code] : undefined
}
export function connectionErrorMessage(error: unknown): string {
  if (error instanceof ConnectionFailure) {
    if (error.status === 409)
      return "Another sign-in may be active, or your environment changed. Check its status; no new sign-in was started automatically."
    if (error.status === 429)
      return "Too many requests. Wait a moment, then check again."
    const problem = connectionProblem(error.code)
    if (problem) return problem
  }
  return "The Codex account status could not be confirmed. Check again to retry."
}

// The private client disables HTTP caching and handles session invalidation centrally.
export async function readConnection(signal: AbortSignal): Promise<Connection> {
  const { data, error, response } = await apiClient.GET(
    "/api/agent-connections/codex",
    {
      signal: AbortSignal.any([signal, AbortSignal.timeout(60_000)]),
    }
  )
  if (!response.ok) throw new ConnectionFailure(response.status, error?.code)
  return connectionSchema.parse(data)
}
export async function changeConnection(
  action: ConnectionAction,
  csrfToken: string,
  signal: AbortSignal
): Promise<Connection> {
  const options = {
    params: { header: { "X-CSRF-Token": csrfToken } },
    signal: AbortSignal.any([signal, AbortSignal.timeout(90_000)]),
  }
  const result =
    action.type === "cancel"
      ? await apiClient.POST("/api/agent-connections/codex/login/cancel", {
          ...options,
          body: { operationId: action.operationId },
        })
      : await apiClient.POST(
          `/api/agent-connections/codex/${action.type}`,
          options
        )
  if (!result.response.ok)
    throw new ConnectionFailure(result.response.status, result.error?.code)
  return connectionSchema.parse(result.data)
}
