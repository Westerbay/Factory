import { z } from "zod"

import { apiClient, ApiFailure } from "../../lib/api/client"
import { safeText } from "./api"

export const claudeSchema = z
  .object({
    provider: z.literal("claude"),
    status: z.enum(["unavailable", "disconnected", "connected", "error"]),
    account: z
      .object({ email: safeText(320).nullish(), plan: safeText(64) })
      .nullish(),
    challenge: z.null().optional(),
    errorCode: z.string().max(100).nullish(),
    observedAt: z.iso.datetime({ offset: true }).nullish(),
  })
  .refine((value) => (value.status === "connected") === Boolean(value.account))

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
