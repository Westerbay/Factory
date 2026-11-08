// Browser credential policy follows WebApp Template; see LICENSE and docs/REUSE.md.
import { z } from "zod"
import createClient from "openapi-fetch"

import { parseApiOrigin } from "./config"
import type { paths } from "./schema"

const configuredOrigin = parseApiOrigin(import.meta.env.VITE_API_BASE_URL)
export const API_ORIGIN =
  configuredOrigin === "/" ? window.location.origin : configuredOrigin
const livenessSchema = z.object({ status: z.literal("ok") })
const authorizationListeners = new Set<(status: number) => void>()

// Browsers control Origin and add it to cross-origin requests; JavaScript cannot
// set it. Keep it required in OpenAPI for other clients while omitting only that
// transport-managed header from the browser client's input types.
type BrowserParameters<P> = P extends { header: infer H }
  ? Omit<P, "header"> & { header: Omit<H, "Origin"> }
  : P
type BrowserOperation<O> = O extends { parameters: infer P }
  ? Omit<O, "parameters"> & { parameters: BrowserParameters<P> }
  : O
type BrowserPaths = {
  [P in keyof paths]: { [M in keyof paths[P]]: BrowserOperation<paths[P][M]> }
}

export const apiClient = createClient<BrowserPaths>({
  baseUrl: API_ORIGIN,
  credentials: "include",
  cache: "no-store",
  redirect: "error",
})

apiClient.use({
  onResponse({ request, response }) {
    const path = new URL(request.url).pathname
    if (
      [401, 403].includes(response.status) &&
      path !== "/api/session" &&
      path !== "/api/auth/logout"
    ) {
      for (const listener of authorizationListeners) listener(response.status)
    }
    return response
  },
})

export function onAuthorizationFailure(listener: (status: number) => void) {
  authorizationListeners.add(listener)
  return () => {
    authorizationListeners.delete(listener)
  }
}

export class ApiFailure extends Error {
  constructor(public readonly status: number) {
    super("The request could not be completed")
  }
}

export async function readLiveness(signal: AbortSignal) {
  const response = await fetch(`${API_ORIGIN}/health/live`, {
    credentials: "include",
    cache: "no-store",
    redirect: "error",
    signal: AbortSignal.any([signal, AbortSignal.timeout(5000)]),
  })
  if (!response.ok) throw new Error("Factory service is unavailable")
  return livenessSchema.parse(await response.json())
}
