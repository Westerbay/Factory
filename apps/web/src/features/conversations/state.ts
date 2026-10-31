import type { Conversation, History } from "./api"

/** Only the latest issued read may update a conversation snapshot. */
export function latestReader<T>(
  read: (id: string, signal: AbortSignal) => Promise<T>
) {
  let generation = 0
  return {
    invalidate() {
      generation++
    },
    async read(id: string, signal: AbortSignal): Promise<T | undefined> {
      const expected = ++generation
      try {
        const value = await read(id, signal)
        return !signal.aborted && expected === generation ? value : undefined
      } catch (error) {
        if (signal.aborted || expected !== generation) return undefined
        throw error
      }
    },
  }
}

/** Each result can update its part of the UI even when the other request fails. */
export function loadWorkspaceResources<T, U>(options: {
  signal: AbortSignal
  list: (signal: AbortSignal) => Promise<T>
  models: (signal: AbortSignal) => Promise<U>
  onList: (value: T) => void
  onModels: (value: U) => void
  onListError: (error: unknown) => void
  onModelsError: (error: unknown) => void
}) {
  const load = async <V>(
    request: (signal: AbortSignal) => Promise<V>,
    onSuccess: (value: V) => void,
    onError: (error: unknown) => void
  ) => {
    try {
      const value = await request(options.signal)
      if (!options.signal.aborted) onSuccess(value)
    } catch (error) {
      if (!options.signal.aborted) onError(error)
    }
  }
  return Promise.all([
    load(options.list, options.onList, options.onListError),
    load(options.models, options.onModels, options.onModelsError),
  ])
}

export function runOutcome(
  history: History | undefined,
  available: boolean
): string {
  if (!available) return "Environment stopped. Start it to continue."
  const run = history?.runs[history.runs.length - 1]
  switch (run?.status) {
    case "active":
      return "Codex is working…"
    case "completed":
      return "Codex finished."
    case "failed":
      return "Codex could not complete this run. Review the conversation and try again."
    case "interrupted":
      return "The run was interrupted. You can start another message."
    case "uncertain":
      return "The run may still be active. Stop the environment and check Recovery before continuing."
    default:
      return ""
  }
}

/** A written answer is not a provider confirmation. Never offer a new response while waiting. */
export function questionReceipt(
  state: "pending" | "claimed" | "uncertain",
  sent: boolean
) {
  if (state === "uncertain")
    return {
      disabled: true,
      message:
        "Delivery could not be confirmed. Stop and recover the run if it remains blocked.",
    }
  if (state === "claimed" || sent)
    return {
      disabled: true,
      message: "Answer sent. Waiting for Codex to confirm receipt.",
    }
  return { disabled: false, message: "" }
}

/** Preserve selection and avoid duplicate options if new chats arrive between pages. */
export function mergeOlderConversations(
  existing: Conversation[],
  older: Conversation[]
) {
  const seen = new Set(existing.map((item) => item.id))
  return [...existing, ...older.filter((item) => !seen.has(item.id))]
}
