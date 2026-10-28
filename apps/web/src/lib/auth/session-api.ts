import { z } from "zod"

import { apiClient, ApiFailure } from "../api/client"
import type { components } from "../api/schema"

export type Session = components["schemas"]["SessionResponse"]

// Validate the security-bearing response at the browser boundary; the generated
// transport type remains the compile-time source of truth for its shape.
const sessionSchema: z.ZodType<Session> = z.object({
  user: z.object({
    id: z.uuid(),
    displayName: z.string(),
    handle: z.string(),
    role: z.enum(["member", "admin"]),
  }),
  csrfToken: z.string().min(1),
  expiresAt: z.iso.datetime({ offset: true }),
})

export async function readSession(signal: AbortSignal): Promise<Session> {
  const { data, response } = await apiClient.GET("/api/session", {
    signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
  })
  if (!response.ok) throw new ApiFailure(response.status)
  return sessionSchema.parse(data)
}

export async function revokeSession(csrfToken: string): Promise<void> {
  const { response } = await apiClient.POST("/api/auth/logout", {
    params: { header: { "X-CSRF-Token": csrfToken } },
    signal: AbortSignal.timeout(10_000),
  })
  if (!response.ok) throw new ApiFailure(response.status)
}
