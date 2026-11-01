import { z } from "zod"

import { apiClient, ApiFailure } from "../../lib/api/client"
import { safeText } from "./api"

function isClaudeAuthorizationUrl(value: string): boolean {
  try {
    const url = new URL(value)
    return (
      url.origin === "https://claude.com" &&
      url.pathname === "/cai/oauth/authorize" &&
      !url.username &&
      !url.password &&
      !url.hash &&
      url.searchParams.getAll("redirect_uri").length === 1 &&
      url.searchParams.get("redirect_uri") ===
        "https://platform.claude.com/oauth/code/callback" &&
      url.searchParams.getAll("response_type").length === 1 &&
      url.searchParams.get("response_type") === "code" &&
      url.searchParams.getAll("code_challenge_method").length === 1 &&
      url.searchParams.get("code_challenge_method") === "S256" &&
      url.searchParams.getAll("state").length === 1 &&
      Boolean(url.searchParams.get("state")) &&
      url.searchParams.getAll("code_challenge").length === 1 &&
      Boolean(url.searchParams.get("code_challenge"))
    )
  } catch {
    return false
  }
}

export const claudeSchema = z
  .object({
    provider: z.literal("claude"),
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
        verificationUrl: z.string().max(8192).refine(isClaudeAuthorizationUrl),
        userCode: z.null(),
        expiresAt: z.iso.datetime({ offset: true }),
      })
      .nullish(),
    errorCode: z.string().max(100).nullish(),
    observedAt: z.iso.datetime({ offset: true }).nullish(),
  })
  .refine(
    (value) =>
      (value.status === "connected") === Boolean(value.account) &&
      (value.status === "connecting") === Boolean(value.challenge)
  )

export type ClaudeConnection = z.infer<typeof claudeSchema>

export function claudeConnectionProblem(
  connection: z.infer<typeof claudeSchema> | undefined
): string | null {
  if (connection?.status !== "error" && connection?.status !== "unavailable")
    return null
  if (connection.errorCode === "unsupported_method") {
    return "Claude is using an unsupported sign-in. Connect your personal Claude account."
  }
  return "Claude status could not be checked. Try again."
}

export async function readClaude(signal: AbortSignal) {
  const { data, response } = await apiClient.GET(
    "/api/agent-connections/claude",
    {
      signal: AbortSignal.any([signal, AbortSignal.timeout(60_000)]),
    }
  )
  if (!response.ok) throw new ApiFailure(response.status)
  return claudeSchema.parse(data)
}

export async function logoutClaude(csrfToken: string) {
  const { data, response } = await apiClient.POST(
    "/api/agent-connections/claude/logout",
    {
      params: { header: { "X-CSRF-Token": csrfToken } },
      signal: AbortSignal.timeout(90_000),
    }
  )
  if (!response.ok) throw new ApiFailure(response.status)
  return claudeSchema.parse(data)
}

export async function beginClaude(csrfToken: string) {
  const { data, response } = await apiClient.POST(
    "/api/agent-connections/claude/login",
    {
      params: { header: { "X-CSRF-Token": csrfToken } },
      signal: AbortSignal.timeout(60_000),
    }
  )
  if (!response.ok) throw new ApiFailure(response.status)
  return claudeSchema.parse(data)
}

export async function cancelClaude(csrfToken: string, operationId: string) {
  const { data, response } = await apiClient.POST(
    "/api/agent-connections/claude/login/cancel",
    {
      params: { header: { "X-CSRF-Token": csrfToken } },
      body: { operationId },
      signal: AbortSignal.timeout(35_000),
    }
  )
  if (!response.ok) throw new ApiFailure(response.status)
  return claudeSchema.parse(data)
}

export async function submitClaudeCode(
  csrfToken: string,
  operationId: string,
  code: string
) {
  const { data, response } = await apiClient.POST(
    "/api/agent-connections/claude/login/code",
    {
      params: { header: { "X-CSRF-Token": csrfToken } },
      body: { operationId, code },
      signal: AbortSignal.timeout(20_000),
    }
  )
  if (!response.ok) throw new ApiFailure(response.status)
  return claudeSchema.parse(data)
}
