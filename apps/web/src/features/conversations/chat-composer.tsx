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
        Message assistant
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
        placeholder="Ask anything…"
        disabled={disabled}
        rows={2}
      />
      <div className="chat-composer-actions">
        <label className="chat-model">
          <span className="sr-only">Model</span>
          <select
            value={model}
            onChange={(event) => onModel(event.target.value)}
            disabled={disabled || !models.length}
          >
            {models.length === 0 && (
              <option value="">No models available</option>
            )}
            {models.map((item) => (
              <option key={item.id} value={item.id}>
                {item.name}
              </option>
            ))}
          </select>
        </label>
        {selected && selected.effortLevels.length > 0 && (
          <label className="chat-effort">
            <span className="sr-only">Reasoning effort</span>
            <select
              value={effort}
              onChange={(event) => onEffort(event.target.value)}
              disabled={disabled}
            >
              {selected.effortLevels.map((level) => (
                <option key={level} value={level}>
                  {level === "ultra" ? "Ultra (automatic delegation)" : level}
                </option>
              ))}
            </select>
          </label>
        )}
        <details className="chat-access">
          <summary
            aria-label="Assistant access information"
            title="Assistant access"
          >
            <CircleHelp aria-hidden="true" size={16} />
            <span className="sr-only">Assistant access</span>
          </summary>
          <p>
            The selected assistant has full access to files and credentials
            inside your environment. It can read, change or delete them without
            asking for each action. Switching providers shares saved messages
            with the selected provider; tool activity and answers to interactive
            questions are not transferred.
          </p>
        </details>
        {running ? (
          <IconButton
            label="Stop response"
            onClick={onStop}
            disabled={stopDisabled}
          >
            <Square aria-hidden="true" size={16} />
          </IconButton>
        ) : (
          <IconButton
            type="submit"
            variant="primary"
            label="Send message"
            disabled={sendDisabled}
          >
            <ArrowUp aria-hidden="true" size={16} />
          </IconButton>
        )}
      </div>
    </form>
  )
}
