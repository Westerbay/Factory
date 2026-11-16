import { z } from "zod"

import { apiClient, API_ORIGIN, ApiFailure } from "../../lib/api/client"
import type { components } from "../../lib/api/schema"

export type Attachment = components["schemas"]["AttachmentView"]

/** Mirrors the limits enforced by the API; the server remains authoritative. */
export const maxAttachmentBytes = 50 * 1024 * 1024
export const maxAttachmentsPerMessage = 10

export const attachmentSchema = z.object({
  id: z.uuid(),
  conversationId: z.uuid(),
  runId: z.uuid().nullish(),
  messageId: z.uuid().nullish(),
  source: z.enum(["user", "agent"]),
  state: z.enum(["uploading", "staged", "attached", "unavailable"]),
  name: z.string(),
  mediaType: z.string(),
  kind: z.enum(["image", "svg", "video", "pdf", "text", "html", "file"]),
  size: z.number().int().nonnegative(),
  createdAt: z.string(),
})

function path(conversation: string, attachment?: string) {
  const base = `/api/conversations/${encodeURIComponent(conversation)}/attachments`
  return attachment ? `${base}/${encodeURIComponent(attachment)}` : base
}

/** Authenticated read URL; cookies authorize it, so it is safe in img, video and iframe. */
export function contentUrl(
  conversation: string,
  attachment: string,
  download = false
) {
  return `${API_ORIGIN}${path(conversation, attachment)}/content${download ? "?download=1" : ""}`
}

export async function upload(
  conversation: string,
  file: File,
  token: string,
  signal: AbortSignal
): Promise<Attachment> {
  const response = await fetch(
    `${API_ORIGIN}${path(conversation)}?name=${encodeURIComponent(file.name)}`,
    {
      method: "POST",
      credentials: "include",
      cache: "no-store",
      redirect: "error",
      headers: {
        "X-CSRF-Token": token,
        "Content-Type": "application/octet-stream",
      },
      body: file,
      signal,
    }
  )
  if (!response.ok) throw new ApiFailure(response.status)
  return attachmentSchema.parse(await response.json())
}

export async function remove(
  conversation: string,
  attachment: string,
  token: string,
  signal: AbortSignal
) {
  const result = await apiClient.DELETE(
    "/api/conversations/{id}/attachments/{attachment}",
    {
      params: {
        path: { id: conversation, attachment },
        header: { "X-CSRF-Token": token },
      },
      signal,
    }
  )
  if (!result.response.ok) throw new ApiFailure(result.response.status)
}

/** Loads at most `limit` bytes of text through a Range request. */
export async function loadText(
  conversation: string,
  attachment: string,
  limit: number,
  signal: AbortSignal
): Promise<string> {
  const response = await fetch(contentUrl(conversation, attachment), {
    credentials: "include",
    cache: "no-store",
    redirect: "error",
    headers: { Range: `bytes=0-${limit - 1}` },
    signal,
  })
  if (!response.ok) throw new ApiFailure(response.status)
  return new TextDecoder().decode(
    new Uint8Array(await response.arrayBuffer()).slice(0, limit)
  )
}

const block = /^```factory-files[ \t]*\r?\n[\s\S]*?^```[ \t]*(?:\r?\n|$)/gm
const openBlock = /^```factory-files[ \t]*(?:\r?\n[\s\S]*)?$/m

/** Removes the machine-readable file list from text shown to people, including a partial one while streaming. */
export function hideFactoryFiles(text: string): string {
  const closed = text.replace(block, "").replace(/\n{3,}/g, "\n\n")
  const match = openBlock.exec(closed)
  return match ? closed.slice(0, match.index) : closed
}
