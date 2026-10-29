import { z } from "zod"

import { apiClient, ApiFailure } from "../../lib/api/client"
import type { components } from "../../lib/api/schema"

export type Environment = components["schemas"]["EnvironmentResponse"]
export type EnvironmentAction = "start" | "stop" | "heartbeat"

const problemMessages: Record<string, string> = {
  volume_missing:
    "Your saved storage could not be found. Contact your administrator; it will not be replaced automatically.",
  provisioning_uncertain:
    "Environment setup was interrupted and needs administrator repair.",
  ownership_conflict:
    "The environment could not be verified. Contact your administrator.",
  policy_mismatch:
    "The environment configuration needs administrator attention.",
  invalid_configuration:
    "Environments need administrator configuration before they can be used.",
  timeout:
    "The environment operation timed out. Refresh to check its current status.",
}

export function environmentProblem(code: string | null | undefined) {
  return code && Object.hasOwn(problemMessages, code)
    ? problemMessages[code]
    : undefined
}

const environmentSchema: z.ZodType<Environment> = z.object({
  status: z.enum([
    "not_created",
    "starting",
    "running",
    "stopping",
    "stopped",
    "error",
    "unavailable",
  ]),
  observedAt: z.iso.datetime({ offset: true }).nullish(),
  idleExpiresAt: z.iso.datetime({ offset: true }).nullish(),
  errorCode: z.string().max(100).nullish(),
})

export class EnvironmentFailure extends ApiFailure {
  constructor(
    status: number,
    public readonly code?: string
  ) {
    super(status)
  }
}

export async function readEnvironment(
  signal: AbortSignal
): Promise<Environment> {
  const { data, response } = await apiClient.GET("/api/environment", {
    signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
  })
  if (!response.ok) throw new EnvironmentFailure(response.status)
  return environmentSchema.parse(data)
}

export async function changeEnvironment(
  action: EnvironmentAction,
  csrfToken: string,
  signal: AbortSignal
): Promise<Environment> {
  const { data, error, response } = await apiClient.POST(
    `/api/environment/${action}`,
    {
      params: { header: { "X-CSRF-Token": csrfToken } },
      // The browser supplies Origin; these routes require a genuinely empty body.
      signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
    }
  )
  if (!response.ok) throw new EnvironmentFailure(response.status, error?.code)
  return environmentSchema.parse(data)
}

export function environmentErrorMessage(error: unknown): string {
  if (error instanceof EnvironmentFailure) {
    const problem = environmentProblem(error.code)
    if (problem) return problem
    if (error.code === "capacity_reached")
      return "All environment slots are in use. Try starting again later."
    if (error.status === 409)
      return "The environment changed or its idle lease expired. Refresh its status before trying again."
    if (error.status === 429)
      return "Too many requests. Wait a moment, then refresh."
    if (error.status === 503)
      return "The environment service is unavailable. Refresh to try again."
  }
  return "The environment status could not be confirmed. Refresh to try again."
}
