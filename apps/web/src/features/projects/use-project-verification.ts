import { useEffect, useMemo, useState } from "react"

import { verifyAddress } from "./api"

type Verification = "checking" | "matched" | "changed" | "unverified"

/** Provider verification is advisory and never blocks local project access. */
export function useProjectVerification(
  projectId: string | undefined,
  available: boolean,
  context?: string
): Verification {
  const selection = useMemo(() => ({}), [projectId, available, context])
  const [result, setResult] = useState<{
    selection: object
    status: Verification
  }>()
  useEffect(() => {
    if (!projectId || !available) return
    const controller = new AbortController()
    void verifyAddress(
      projectId,
      AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)])
    )
      .then((status) => {
        if (!controller.signal.aborted) setResult({ selection, status })
      })
      .catch(() => {
        if (!controller.signal.aborted)
          setResult({ selection, status: "unverified" })
      })
    return () => controller.abort()
  }, [projectId, available, selection])
  return result?.selection === selection
    ? result.status
    : available
      ? "checking"
      : "unverified"
}
