import { useTranslation } from "react-i18next"
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
  const { t: translate, i18n } = useTranslation()
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
      aria-label={headingRef ? undefined : translate("Claude sign-in")}
    >
      {headingRef && (
        <h1 ref={headingRef} tabIndex={-1}>
          {translate("Connect Claude Code")}{" "}
        </h1>
      )}
      {connection.pending === "login" && (
        <p role="status">{translate("Starting Claude sign-in…")}</p>
      )}
      {challenge && (
        <div className="provider-device-code">
          <p>
            {translate(
              "Open Claude, sign in, then paste the code Claude gives you below."
            )}{" "}
          </p>
          <a
            className="ui-button ui-button-primary provider-continue"
            href={challenge.verificationUrl}
            target="_blank"
            rel="noopener noreferrer"
          >
            {translate("Continue to Claude")}{" "}
            <ArrowUpRight aria-hidden="true" />
            <span className="sr-only"> {translate("(opens a new tab)")}</span>
          </a>
          {!connection.submitted ? (
            <form className="provider-auth-form" onSubmit={submit}>
              <label htmlFor={inputId}>{translate("Code from Claude")}</label>
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
                {translate("Connect Claude")}{" "}
              </Button>
            </form>
          ) : (
            <p role="status">
              {translate("Waiting for Claude to confirm sign-in…")}
            </p>
          )}
          <p className="provider-expiry">
            {translate("Expires at")}{" "}
            <time dateTime={challenge.expiresAt}>
              {new Date(challenge.expiresAt).toLocaleTimeString(i18n.language)}
            </time>
          </p>
        </div>
      )}
      {problem && (
        <p className="provider-status" role="alert">
          {translate(problem)}
        </p>
      )}
      {!challenge && !busy && (
        <Button variant="secondary" onClick={() => void connection.begin()}>
          {translate("Try again")}{" "}
        </Button>
      )}
      <Button
        variant="ghost"
        className="provider-cancel"
        disabled={busy}
        onClick={() => void close()}
      >
        {translate("Back")}{" "}
      </Button>
    </section>
  )
}
