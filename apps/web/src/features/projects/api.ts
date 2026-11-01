import { z } from "zod"

import { apiClient, ApiFailure } from "../../lib/api/client"
import type { components } from "../../lib/api/schema"
import { listRecoverable as listRecoverableRuns } from "../conversations/api"

const id = z.uuid()
const projectSchema = z.object({
  id,
  fullName: z.string().min(1).max(256),
  state: z.enum(["importing", "uncertain", "ready", "failed", "deleting"]),
  renamedSinceClone: z.boolean(),
})
const projectPageSchema = z.object({
  items: z.array(projectSchema),
  nextCursor: id.nullish(),
})
const repositorySchema = z.object({
  id: z.number().int().positive().safe(),
  fullName: z.string().min(1).max(256),
  private: z.boolean(),
})
const repositoryPageSchema = z.object({
  items: z.array(repositorySchema),
  nextPage: z.number().int().positive().nullish(),
})
const connectionSchema = z.object({
  status: z.enum(["connected", "disconnected"]),
  login: z.string().nullish(),
  email: z.string().nullish(),
})
const recoverableSchema = z.array(z.object({ projectId: id }))
const verificationSchema = z.object({ status: z.enum(["matched", "changed"]) })
const authorizationSchema = z.object({ authorizationUrl: z.url() })
const apiErrorSchema = z.object({ code: z.string() })

export type Project = z.infer<typeof projectSchema>
export type Repository = z.infer<typeof repositorySchema>
export type ProjectConnection = z.infer<typeof connectionSchema>
export type RecoverableImport = z.infer<typeof recoverableSchema>[number]
export type ProjectPage = z.infer<typeof projectPageSchema>
export type RepositoryPage = z.infer<typeof repositoryPageSchema>

export class ProjectFailure extends ApiFailure {
  constructor(
    status: number,
    public readonly code: string
  ) {
    super(status)
  }
}
/** These responses reject before the import is reserved; transport errors remain ambiguous. */
export function isDefiniteImportRejection(error: unknown): boolean {
  return (
    error instanceof ProjectFailure &&
    [
      "project_directory_exists",
      "provider_unauthorized",
      "provider_forbidden",
      "provider_rate_limited",
      "email_unavailable",
      "invalid_github_identity",
      "forbidden",
    ].includes(error.code)
  )
}
function checked<T>(result: {
  data?: T
  error?: unknown
  response: Response
}): T {
  if (!result.response.ok) {
    const error = apiErrorSchema.safeParse(result.error)
    throw new ProjectFailure(
      result.response.status,
      error.success ? error.data.code : "unavailable"
    )
  }
  if (result.data === undefined) throw new Error("Missing response")
  return result.data
}
function check(result: { error?: unknown; response: Response }) {
  if (!result.response.ok) {
    const error = apiErrorSchema.safeParse(result.error)
    throw new ProjectFailure(
      result.response.status,
      error.success ? error.data.code : "unavailable"
    )
  }
}
const csrf = (token: string) => ({ "X-CSRF-Token": token })

export async function connection(
  signal: AbortSignal
): Promise<ProjectConnection> {
  return connectionSchema.parse(
    checked(await apiClient.GET("/api/projects/connection", { signal }))
  )
}
export async function setup(token: string, signal: AbortSignal) {
  check(
    await apiClient.POST("/api/projects/setup", {
      params: { header: csrf(token) },
      signal,
    })
  )
}
/** Complete setup once the workspace is running; old sessions may need reconnection. */
export async function ensureConnection(
  token: string,
  signal: AbortSignal
): Promise<ProjectConnection> {
  const account = await connection(signal)
  if (account.status === "connected") return account
  try {
    await setup(token, signal)
    return await connection(signal)
  } catch (error) {
    if (
      error instanceof ProjectFailure &&
      error.code === "provider_unauthorized"
    )
      return account
    throw error
  }
}
export async function connect(
  token: string,
  signal: AbortSignal
): Promise<string> {
  const response = authorizationSchema.parse(
    checked(
      await apiClient.POST("/api/projects/connect", {
        params: { header: csrf(token) },
        signal,
      })
    )
  )
  const url = new URL(response.authorizationUrl)
  if (
    url.protocol !== "https:" ||
    url.hostname !== "github.com" ||
    url.pathname !== "/login/oauth/authorize" ||
    url.port ||
    url.username ||
    url.password
  )
    throw new Error("Invalid authorization URL")
  return url.href
}
export async function disconnect(token: string, signal: AbortSignal) {
  check(
    await apiClient.POST("/api/projects/disconnect", {
      params: { header: csrf(token) },
      signal,
    })
  )
}
export async function list(
  signal: AbortSignal,
  before?: string
): Promise<ProjectPage> {
  return projectPageSchema.parse(
    checked(
      await apiClient.GET("/api/projects", {
        params: { query: before ? { before } : {} },
        signal,
      })
    )
  )
}
export async function get(
  projectId: string,
  signal: AbortSignal
): Promise<Project> {
  return projectSchema.parse(
    checked(
      await apiClient.GET("/api/projects/{id}", {
        params: { path: { id: projectId } },
        signal,
      })
    )
  )
}
export async function remove(
  projectId: string,
  token: string,
  signal: AbortSignal
) {
  check(
    await apiClient.DELETE("/api/projects/{id}", {
      params: { path: { id: projectId }, header: csrf(token) },
      signal,
    })
  )
}
export async function verifyAddress(
  projectId: string,
  signal: AbortSignal
): Promise<"matched" | "changed"> {
  return verificationSchema.parse(
    checked(
      await apiClient.GET("/api/projects/{id}/verification", {
        params: { path: { id: projectId } },
        signal,
      })
    )
  ).status
}
export async function repositories(
  page: number,
  token: string,
  signal: AbortSignal
): Promise<RepositoryPage> {
  return repositoryPageSchema.parse(
    checked(
      await apiClient.POST("/api/projects/repositories", {
        params: { header: csrf(token) },
        body: { page },
        signal,
      })
    )
  )
}
export async function importRepository(
  repositoryId: number,
  page: number,
  token: string,
  signal: AbortSignal
): Promise<Project> {
  const body: components["schemas"]["ImportBody"] = { repositoryId, page }
  return projectSchema.parse(
    checked(
      await apiClient.POST("/api/projects", {
        params: { header: csrf(token) },
        body,
        signal,
      })
    )
  )
}
export async function retry(
  projectId: string,
  token: string,
  signal: AbortSignal
): Promise<Project> {
  return projectSchema.parse(
    checked(
      await apiClient.POST("/api/projects/{id}/retry", {
        params: { path: { id: projectId }, header: csrf(token) },
        signal,
      })
    )
  )
}
export async function listRecoverable(
  signal: AbortSignal
): Promise<RecoverableImport[]> {
  return recoverableSchema.parse(
    checked(await apiClient.GET("/api/projects/recoverable", { signal }))
  )
}
/** A consumed page is not evidence that no later page exists. */
export async function remainingAfterRecovery(
  remaining: RecoverableImport[],
  signal: AbortSignal
): Promise<RecoverableImport[]> {
  return remaining.length ? remaining : listRecoverable(signal)
}
/** Any unavailable recovery source blocks an environment restart. */
export async function hasRecoverableWork(
  signal: AbortSignal
): Promise<boolean> {
  const [runs, imports] = await Promise.all([
    listRecoverableRuns(signal),
    listRecoverable(signal),
  ])
  return runs.length > 0 || imports.length > 0
}
export async function recover(
  projectId: string,
  token: string,
  signal: AbortSignal
) {
  check(
    await apiClient.POST("/api/projects/{id}/recover", {
      params: { path: { id: projectId }, header: csrf(token) },
      signal,
    })
  )
}

export function projectProblem(error: unknown): string {
  if (!(error instanceof ProjectFailure))
    return "Projects are unavailable. Try again."
  switch (error.code) {
    case "provider_unauthorized":
      return "GitHub repository access expired. Reconnect GitHub."
    case "provider_forbidden":
      return "GitHub denied access. Check your organization permissions or SSO."
    case "provider_rate_limited":
      return "GitHub rate limit reached. Try again later."
    case "email_unavailable":
      return "GitHub did not return a verified primary email. Verify your GitHub email, then reconnect."
    case "invalid_github_identity":
      return "The GitHub account does not match this Factory account."
    case "project_in_use":
      return "Stop the project’s active or uncertain runs and complete import recovery before deleting."
    case "project_directory_exists":
      return "A file or folder with this repository name already exists in Documents. Rename or remove it before importing."
    case "conflict":
      return "The project or environment changed. Check its status and try again."
    default:
      return "Projects are unavailable. Try again."
  }
}
