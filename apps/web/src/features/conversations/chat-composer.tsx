import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import { useId, useRef } from "react"
import { ArrowUp, CircleHelp, Square } from "lucide-react"

import { IconButton } from "../../components/ui/button"
import type { Model } from "./api"

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
}) {
  useLocale()
  const inputId = useId()
  const composing = useRef(false)
  const selected = models.find((item) => item.id === model)
  return (
    <form
      className="chat-composer"
      onSubmit={(event) => {
        event.preventDefault()
        if (!sendDisabled && !running) onSend()
      }}
    >
      <label className="sr-only" htmlFor={inputId}>
        {m.message_assistant()}{" "}
      </label>
      <textarea
        id={inputId}
        value={input}
        onChange={(event) => onInput(event.target.value)}
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
        {selected && selected.effortLevels.length > 0 && (
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
            {m.the_selected_assistant_has_full_access_to_files_and_credentials_i()}{" "}
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
