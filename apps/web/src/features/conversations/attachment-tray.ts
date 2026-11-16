import {
  maxAttachmentBytes,
  maxAttachmentsPerMessage,
  type Attachment,
} from "./attachments"

export type TrayItem = {
  key: string
  name: string
  size: number
  status: "uploading" | "ready" | "failed"
  attachment?: Attachment
}

export type TrayRejection = {
  name: string
  reason: "too-large" | "too-many"
}

export type TrayDependencies = {
  /** Resolves the conversation, creating it first when the composer has none. */
  conversation: () => Promise<string>
  upload: (
    conversation: string,
    file: File,
    signal: AbortSignal
  ) => Promise<Attachment>
  remove: (conversation: string, attachment: string) => Promise<void>
}

export type TraySnapshot = {
  items: TrayItem[]
  rejections: TrayRejection[]
}

/** In-memory composer tray: a staged upload belongs to one unsent message. */
export function createAttachmentTray(deps: TrayDependencies) {
  let items: TrayItem[] = []
  let rejections: TrayRejection[] = []
  let snapshot: TraySnapshot = { items, rejections }
  const aborts = new Map<string, AbortController>()
  const listeners = new Set<() => void>()
  let sequence = 0

  function publish() {
    snapshot = { items, rejections }
    for (const listener of listeners) listener()
  }
  function update(key: string, change: Partial<TrayItem>) {
    if (!items.some((item) => item.key === key)) return
    items = items.map((item) =>
      item.key === key ? { ...item, ...change } : item
    )
    publish()
  }
  async function send(key: string, file: File, controller: AbortController) {
    try {
      const conversation = await deps.conversation()
      const attachment = await deps.upload(
        conversation,
        file,
        controller.signal
      )
      if (controller.signal.aborted) {
        // Removed while in flight: do not leave the staged file behind.
        await deps.remove(conversation, attachment.id).catch(() => undefined)
        return
      }
      update(key, { status: "ready", attachment })
    } catch {
      if (!controller.signal.aborted) update(key, { status: "failed" })
    } finally {
      aborts.delete(key)
    }
  }

  return {
    subscribe(listener: () => void) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    getSnapshot: () => snapshot,
    /** Ready attachments to send with the message, in the order they were added. */
    readyIds: () =>
      items.flatMap((item) =>
        item.status === "ready" && item.attachment ? [item.attachment.id] : []
      ),
    uploading: () => items.some((item) => item.status === "uploading"),
    add(files: readonly File[]) {
      const rejected: TrayRejection[] = []
      const accepted: { key: string; file: File }[] = []
      let counted = items.filter((item) => item.status !== "failed").length
      for (const file of files) {
        if (file.size > maxAttachmentBytes)
          rejected.push({ name: file.name, reason: "too-large" })
        else if (counted >= maxAttachmentsPerMessage)
          rejected.push({ name: file.name, reason: "too-many" })
        else {
          counted++
          accepted.push({ key: `attachment-${++sequence}`, file })
        }
      }
      rejections = rejected
      items = [
        ...items,
        ...accepted.map(({ key, file }) => ({
          key,
          name: file.name,
          size: file.size,
          status: "uploading" as const,
        })),
      ]
      publish()
      for (const { key, file } of accepted) {
        const controller = new AbortController()
        aborts.set(key, controller)
        void send(key, file, controller)
      }
    },
    /** Removes an item; a staged upload is also deleted from the server. */
    async remove(key: string) {
      const item = items.find((entry) => entry.key === key)
      if (!item) return
      aborts.get(key)?.abort()
      items = items.filter((entry) => entry.key !== key)
      publish()
      if (item.attachment)
        await deps
          .remove(item.attachment.conversationId, item.attachment.id)
          .catch(() => undefined)
    },
    dismissRejections() {
      rejections = []
      publish()
    },
    /** Forgets sent items without deleting them: the message now owns them. */
    clear() {
      for (const controller of aborts.values()) controller.abort()
      aborts.clear()
      items = []
      rejections = []
      publish()
    },
  }
}

export type AttachmentTray = ReturnType<typeof createAttachmentTray>
