import * as m from "@workspace/i18n/messages"
import { useCallback, useMemo, useRef, useState } from "react"
import { File as FileIcon, FileText, Film } from "lucide-react"

import { Button } from "../../components/ui/button"
import { Modal } from "../../components/ui/modal"
import { useLocale } from "../../lib/i18n"
import { MediaView, type MediaKind } from "../artifacts/media"
import { contentUrl, loadText, type Attachment } from "./attachments"

const thumbnailKinds: MediaKind[] = ["image", "svg"]

function useMediaLabels() {
  useLocale()
  return {
    download: m.attachment_download(),
    loading: m.attachment_viewer_loading(),
    error: m.attachment_viewer_error(),
    tooLarge: m.attachment_viewer_too_large(),
  }
}

function Viewer({
  attachment,
  onClose,
  returnFocusRef,
}: {
  attachment: Attachment
  onClose: () => void
  returnFocusRef: React.RefObject<HTMLElement | null>
}) {
  const labels = useMediaLabels()
  const { id, conversationId } = attachment
  const source = useMemo(
    () => ({
      kind: attachment.kind,
      name: attachment.name,
      size: attachment.size,
      contentUrl: contentUrl(conversationId, id),
      downloadUrl: contentUrl(conversationId, id, true),
      loadText: (limit: number, signal: AbortSignal) =>
        loadText(conversationId, id, limit, signal),
    }),
    [attachment.kind, attachment.name, attachment.size, conversationId, id]
  )
  return (
    <Modal
      open
      onClose={onClose}
      title={attachment.name}
      returnFocusRef={returnFocusRef}
      className="attachment-viewer"
    >
      <MediaView source={source} labels={labels} />
      <div className="attachment-viewer-actions">
        <a
          className="ui-button"
          href={source.downloadUrl}
          download={attachment.name}
          rel="noreferrer"
        >
          {m.attachment_download()}
        </a>
        <Button onClick={onClose}>{m.close()}</Button>
      </div>
    </Modal>
  )
}

function KindIcon({ kind }: { kind: MediaKind }) {
  if (kind === "video") return <Film aria-hidden="true" size={16} />
  if (kind === "file") return <FileIcon aria-hidden="true" size={16} />
  return <FileText aria-hidden="true" size={16} />
}

function Item({ attachment }: { attachment: Attachment }) {
  useLocale()
  const [open, setOpen] = useState(false)
  const trigger = useRef<HTMLButtonElement>(null)
  const close = useCallback(() => setOpen(false), [])
  const unavailable = attachment.state !== "attached"
  if (unavailable)
    return (
      <li
        className="attachment-chip"
        data-state="unavailable"
        title={m.attachment_unavailable_detail({ name: attachment.name })}
      >
        <KindIcon kind={attachment.kind} />
        <span className="attachment-name">{attachment.name}</span>
        <small>{m.attachment_unavailable()}</small>
      </li>
    )
  const thumbnail = thumbnailKinds.includes(attachment.kind)
  const download = attachment.kind === "file"
  return (
    <li className={thumbnail ? "attachment-thumbnail" : "attachment-chip"}>
      {download ? (
        <a
          href={contentUrl(attachment.conversationId, attachment.id, true)}
          download={attachment.name}
          rel="noreferrer"
          aria-label={`${m.attachment_download()} · ${attachment.name}`}
        >
          <KindIcon kind={attachment.kind} />
          <span className="attachment-name">{attachment.name}</span>
        </a>
      ) : (
        <button
          type="button"
          ref={trigger}
          onClick={() => setOpen(true)}
          aria-label={m.attachment_open({ name: attachment.name })}
        >
          {thumbnail ? (
            <img
              src={contentUrl(attachment.conversationId, attachment.id)}
              alt={attachment.name}
              loading="lazy"
              referrerPolicy="no-referrer"
            />
          ) : (
            <>
              <KindIcon kind={attachment.kind} />
              <span className="attachment-name">{attachment.name}</span>
            </>
          )}
        </button>
      )}
      {open && (
        <Viewer
          attachment={attachment}
          onClose={close}
          returnFocusRef={trigger}
        />
      )}
    </li>
  )
}

/** Attachments of one message: image thumbnails first, then generic file chips. */
export function AttachmentList({ attachments }: { attachments: Attachment[] }) {
  useLocale()
  if (attachments.length === 0) return null
  const ordered = [
    ...attachments.filter((item) => thumbnailKinds.includes(item.kind)),
    ...attachments.filter((item) => !thumbnailKinds.includes(item.kind)),
  ]
  return (
    <ul className="attachment-list" aria-label={m.attachment_files()}>
      {ordered.map((item) => (
        <Item key={item.id} attachment={item} />
      ))}
    </ul>
  )
}
