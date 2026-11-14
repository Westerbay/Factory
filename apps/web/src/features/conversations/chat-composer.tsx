import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import { useId, useRef, useState, useSyncExternalStore } from "react"
import { ArrowUp, CircleHelp, Paperclip, Square, X } from "lucide-react"

import { IconButton } from "../../components/ui/button"
import type { Model } from "./api"
import type { AttachmentTray } from "./attachment-tray"

function noopSubscribe() {
  return () => {}
}
const emptyTray = { items: [], rejections: [] }
const emptySnapshot = () => emptyTray

export function ChatComposer({
  input,
  model,
  models,
  effort,
  disabled,
  sendDisabled,
  running,
  stopDisabled,
  onInput,
  onModel,
  onEffort,
  onSend,
  onStop,
  workflow,
  tray,
}: {
  input: string
  model: string
  models: Model[]
  effort: string
  disabled: boolean
  sendDisabled: boolean
  running: boolean
  stopDisabled: boolean
  onInput: (value: string) => void
  onModel: (value: string) => void
  onEffort: (value: string) => void
  onSend: () => void
  onStop: () => void
  workflow?: { modelLabel: string; access: string }
  tray?: AttachmentTray
}) {
  useLocale()
  const inputId = useId()
  const composing = useRef(false)
  const selected = models.find((item) => item.id === model)
  const files = useRef<HTMLInputElement>(null)
  const [dragging, setDragging] = useState(false)
  const { items, rejections } = useSyncExternalStore(
    tray?.subscribe ?? noopSubscribe,
    tray?.getSnapshot ?? emptySnapshot
  )
  const attach = (list: Iterable<File>) => {
    const added = [...list]
    if (!disabled && tray && added.length) tray.add(added)
  }
  const hasFiles = (event: React.DragEvent) =>
    event.dataTransfer.types.includes("Files")
  return (
    <form
      className="chat-composer"
      data-dragging={dragging}
      onDragOver={(event) => {
        if (!hasFiles(event)) return
        event.preventDefault()
        if (tray && !disabled) setDragging(true)
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null))
          setDragging(false)
      }}
      onDrop={(event) => {
        if (!hasFiles(event)) return
        event.preventDefault()
        setDragging(false)
        attach(event.dataTransfer.files)
      }}
      onSubmit={(event) => {
        event.preventDefault()
        if (!sendDisabled && !running) onSend()
      }}
    >
      {(items.length > 0 || rejections.length > 0) && tray && (
        <div>
          <ul className="attachment-tray" aria-label={m.attachment_tray()}>
            {items.map((item) => (
              <li
                key={item.key}
                className="attachment-chip"
                data-status={item.status}
              >
                <span className="attachment-chip-body">
                  <span className="attachment-name">{item.name}</span>
                  <small role="status">
                    {item.status === "uploading"
                      ? m.attachment_uploading()
                      : item.status === "failed"
                        ? m.attachment_failed()
                        : m.attachment_ready()}
                  </small>
                  <button
                    type="button"
                    aria-label={m.attachment_remove({ name: item.name })}
                    onClick={() => void tray.remove(item.key)}
                  >
                    <X aria-hidden="true" size={12} />
                  </button>
                </span>
              </li>
            ))}
          </ul>
          {rejections.length > 0 && (
            <div className="attachment-notice" role="status">
              <span>
                {rejections
                  .map((item) =>
                    item.reason === "too-large"
                      ? m.attachment_too_large({ name: item.name })
                      : m.attachment_too_many({ name: item.name })
                  )
                  .join(" ")}
              </span>
              <button type="button" onClick={() => tray.dismissRejections()}>
                {m.attachment_dismiss()}
              </button>
            </div>
          )}
        </div>
      )}
      <label className="sr-only" htmlFor={inputId}>
        {m.message_assistant()}{" "}
      </label>
      <textarea
        id={inputId}
        value={input}
        onChange={(event) => onInput(event.target.value)}
        onPaste={(event) => {
          const pasted = [...event.clipboardData.files]
          if (!tray || disabled || pasted.length === 0) return
          event.preventDefault()
          attach(pasted)
        }}
        onCompositionStart={() => {
          composing.current = true
        }}
        onCompositionEnd={() => {
          composing.current = false
        }}
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !event.shiftKey &&
            !event.nativeEvent.isComposing &&
            !composing.current &&
            event.nativeEvent.keyCode !== 229
          ) {
            event.preventDefault()
            if (!sendDisabled && !running) onSend()
          }
        }}
        placeholder={m.ask_anything()}
        disabled={disabled}
        rows={2}
      />
      <div className="chat-composer-actions">
        {tray && (
          <>
            <input
              ref={files}
              type="file"
              multiple
              hidden
              tabIndex={-1}
              onChange={(event) => {
                attach(event.target.files ?? [])
                event.target.value = ""
              }}
            />
            <IconButton
              label={m.attachment_add()}
              onClick={() => files.current?.click()}
              disabled={disabled}
            >
              <Paperclip aria-hidden="true" size={16} />
            </IconButton>
          </>
        )}
        {workflow ? (
          <span className="chat-model chat-workflow-model">
            {workflow.modelLabel}
          </span>
        ) : (
          <label className="chat-model">
            <span className="sr-only">{m.model()}</span>
            <select
              value={model}
              onChange={(event) => onModel(event.target.value)}
              disabled={disabled || !models.length}
            >
              {models.length === 0 && (
                <option value="">{m.no_models_available()}</option>
              )}
              {models.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.name.replace(" (1M context)", m.m_1m_context())}
                </option>
              ))}
            </select>
          </label>
        )}
        {!workflow && selected && selected.effortLevels.length > 0 && (
          <label className="chat-effort">
            <span className="sr-only">{m.reasoning_effort()}</span>
            <select
              value={effort}
              onChange={(event) => onEffort(event.target.value)}
              disabled={disabled}
            >
              {selected.effortLevels.map((level) => (
                <option key={level} value={level}>
                  {level === "ultra"
                    ? m.ultra_automatic_delegation()
                    : translateMessage(level)}
                </option>
              ))}
            </select>
          </label>
        )}
        <details className="chat-access">
          <summary
            aria-label={m.assistant_access_information()}
            title={m.assistant_access()}
          >
            <CircleHelp aria-hidden="true" size={16} />
            <span className="sr-only">{m.assistant_access()}</span>
          </summary>
          <p>
            {workflow?.access ??
              m.the_selected_assistant_has_full_access_to_files_and_credentials_i()}{" "}
          </p>
        </details>
        {running ? (
          <IconButton
            label={m.stop_response()}
            onClick={onStop}
            disabled={stopDisabled}
          >
            <Square aria-hidden="true" size={16} />
          </IconButton>
        ) : (
          <IconButton
            type="submit"
            variant="primary"
            label={m.send_message()}
            disabled={sendDisabled}
          >
            <ArrowUp aria-hidden="true" size={16} />
          </IconButton>
        )}
      </div>
    </form>
  )
}
