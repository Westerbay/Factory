// Browser credential policy follows WebApp Template; see LICENSE and docs/REUSE.md.
import { z } from "zod"

import { parseApiOrigin } from "./config"

const API_ORIGIN = parseApiOrigin(import.meta.env.VITE_API_BASE_URL)
const livenessSchema = z.object({ status: z.literal("ok") })

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
