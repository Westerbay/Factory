import { z } from "zod"

import { apiClient, ApiFailure } from "../../lib/api/client"
import type { components } from "../../lib/api/schema"

export type AdminUser = components["schemas"]["AdminUser"]
export type UserStatus = components["schemas"]["UserStatus"]
export type AccessAction = components["schemas"]["AccessAction"]
const PAGE_SIZE = 20

const userSchema: z.ZodType<AdminUser> = z.object({
  id: z.uuid(),
  displayName: z.string(),
  handle: z.string(),
  provider: z.string(),
  role: z.enum(["member", "admin"]),
  status: z.enum(["pending", "active", "suspended"]),
  version: z.number().int().nonnegative(),
})
const usersSchema: z.ZodType<components["schemas"]["UsersResponse"]> = z.object(
  {
    items: z.array(userSchema).max(PAGE_SIZE),
    nextCursor: z.uuid().nullish(),
  }
)

export function parseUsersSearch(search: Record<string, unknown>): {
  status?: UserStatus
  cursor?: string
} {
  const status = z
    .enum(["pending", "active", "suspended"])
    .safeParse(search.status)
  const cursor = z.uuid().safeParse(search.cursor)
  return {
    status: status.success ? status.data : undefined,
    cursor: cursor.success ? cursor.data : undefined,
  }
}

export async function listUsers(
  status: UserStatus,
  cursor: string | undefined,
  signal: AbortSignal
) {
  const { data, response } = await apiClient.GET("/api/admin/users", {
    params: { query: { status, cursor, limit: PAGE_SIZE } },
    signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
  })
  if (!response.ok) throw new ApiFailure(response.status)
  return usersSchema.parse(data)
}

export async function changeAccess(
  user: AdminUser,
  status: AccessAction,
  csrfToken: string
) {
  const { data, response } = await apiClient.PATCH(
    "/api/admin/users/{id}/access",
    {
      params: { path: { id: user.id }, header: { "X-CSRF-Token": csrfToken } },
      body: { status, expectedVersion: user.version },
      signal: AbortSignal.timeout(10_000),
    }
  )
  if (!response.ok) throw new ApiFailure(response.status)
  return userSchema.parse(data)
}
