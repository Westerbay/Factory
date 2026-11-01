import type { Project } from "./api"

/** Replace the first page on refresh; only retain later entries after revalidation. */
export function mergeProjectPage(
  current: Project[],
  incoming: Project[],
  first: boolean,
  preserved: Project[] = []
): Project[] {
  if (first) {
    const received = new Set(incoming.map((project) => project.id))
    return [
      ...incoming,
      ...preserved.filter((project) => !received.has(project.id)),
    ]
  }
  const updates = new Map(incoming.map((project) => [project.id, project]))
  const existing = new Set(current.map((project) => project.id))
  return [
    ...current.map((project) => updates.get(project.id) ?? project),
    ...incoming.filter((project) => !existing.has(project.id)),
  ]
}

const callbackMessages: Record<string, string> = {
  connected: "GitHub repository access connected.",
  denied: "GitHub authorization was cancelled. You can try again.",
  "authorization-invalid":
    "GitHub authorization could not be confirmed. Try connecting again.",
  "identity-mismatch": "The GitHub account did not match your Factory account.",
  forbidden: "GitHub denied access. Check organization permissions or SSO.",
  expired: "GitHub authorization expired. Try connecting again.",
  "rate-limited": "GitHub rate limit reached. Try again later.",
  "email-unavailable":
    "GitHub did not return a verified primary email. Verify it, then reconnect.",
  unavailable: "GitHub access could not be confirmed. Try connecting again.",
}

export function githubCallbackMessage(code: string): string {
  return Object.hasOwn(callbackMessages, code)
    ? (callbackMessages[code] ?? "GitHub authorization could not be confirmed.")
    : "GitHub authorization could not be confirmed."
}

export function projectWarnings(
  projectId: string | undefined,
  verification: "checking" | "matched" | "changed" | "unverified",
  renamedSinceClone: boolean
): string[] {
  if (!projectId) return []
  const warnings =
    verification === "matched" || verification === "checking"
      ? []
      : [
          verification === "changed"
            ? "The original GitHub address changed or is inaccessible. Check the Git remote before pushing."
            : "GitHub address not verified; check your Git remote before pushing.",
        ]
  if (renamedSinceClone)
    warnings.push(
      "Repository renamed since import. Verify the Git remote before pushing."
    )
  return warnings
}
