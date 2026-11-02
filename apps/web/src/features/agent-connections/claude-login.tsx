import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import { useId, useRef, type FormEvent, type RefObject } from "react"
import { ArrowUpRight } from "lucide-react"

import { Button } from "../../components/ui/button"
import { claudeConnectionProblem } from "./claude-api"
import { useClaudeConnection } from "./use-claude-connection"

export function ClaudeLogin({
  connection,
  onClose,
  headingRef,
}: {
  connection: ReturnType<typeof useClaudeConnection>
  onClose: () => void
  headingRef?: RefObject<HTMLHeadingElement | null>
}) {
  const locale = useLocale()
  const inputId = useId()
  const input = useRef<HTMLInputElement>(null)
  const challenge = connection.data?.challenge
  const problem = connection.message ?? claudeConnectionProblem(connection.data)
  const busy = connection.pending !== null
  async function close() {
    if (challenge && !(await connection.cancel())) return
    onClose()
  }
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const code = input.current?.value.trim()
    if (!code) return
    if (input.current) input.current.value = ""
    void connection.submit(code)
  }
  return (
    <section
      className="provider-connection"
      aria-label={headingRef ? undefined : m.claude_sign_in()}
    >
      {headingRef && (
        <h1 ref={headingRef} tabIndex={-1}>
          {m.connect_claude_code()}{" "}
        </h1>
      )}
      {connection.pending === "login" && (
        <p role="status">{m.starting_claude_sign_in()}</p>
      )}
      {challenge && (
        <div className="provider-device-code">
          <p>
            {m.open_claude_sign_in_then_paste_the_code_claude_gives_you_below()}{" "}
          </p>
          <a
            className="ui-button ui-button-primary provider-continue"
            href={challenge.verificationUrl}
            target="_blank"
            rel="noopener noreferrer"
          >
            {m.continue_to_claude()} <ArrowUpRight aria-hidden="true" />
            <span className="sr-only"> {m.opens_a_new_tab()}</span>
          </a>
          {!connection.submitted ? (
            <form className="provider-auth-form" onSubmit={submit}>
              <label htmlFor={inputId}>{m.code_from_claude()}</label>
              <input
                ref={input}
                id={inputId}
                type="text"
                autoComplete="off"
                spellCheck={false}
                maxLength={768}
                required
                disabled={busy}
              />
              <Button type="submit" variant="secondary" disabled={busy}>
                {m.connect_claude()}{" "}
              </Button>
            </form>
          ) : (
            <p role="status">{m.waiting_for_claude_to_confirm_sign_in()}</p>
          )}
          <p className="provider-expiry">
            {m.expires_at()}{" "}
            <time dateTime={challenge.expiresAt}>
              {new Date(challenge.expiresAt).toLocaleTimeString(locale)}
            </time>
          </p>
        </div>
      )}
      {problem && (
        <p className="provider-status" role="alert">
          {translateMessage(problem)}
        </p>
      )}
      {!challenge && !busy && (
        <Button variant="secondary" onClick={() => void connection.begin()}>
          {m.try_again()}{" "}
        </Button>
      )}
      <Button
        variant="ghost"
        className="provider-cancel"
        disabled={busy}
        onClick={() => void close()}
      >
        {m.back()}{" "}
      </Button>
    </section>
  )
}
