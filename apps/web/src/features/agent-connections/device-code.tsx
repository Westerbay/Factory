import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import { useEffect, useId, useRef, useState } from "react"
import { ArrowUpRight, Check, Copy } from "lucide-react"

import { Button } from "../../components/ui/button"
import type { LoginChallenge } from "./api"

export function DeviceCode({ challenge }: { challenge: LoginChallenge }) {
  const locale = useLocale()
  const codeId = useId()
  const [copied, setCopied] = useState(false)
  const [message, setMessage] = useState("")
  const mounted = useRef(true)
  const userCode = challenge.userCode
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  async function copy() {
    if (!userCode) return
    try {
      await navigator.clipboard.writeText(userCode)
      if (mounted.current) {
        setCopied(true)
        setMessage("Code copied.")
      }
    } catch {
      if (mounted.current) {
        setCopied(false)
        setMessage("Select the code and copy it manually.")
      }
    }
  }

  if (!userCode) return null

  return (
    <div className="provider-device-code">
      <p>{m.enter_this_code_on_openai_to_connect_your_account()}</p>
      <div className="provider-code-row">
        <span className="sr-only" id={codeId}>
          {m.authorization_code()}{" "}
        </span>
        <code aria-labelledby={codeId}>{userCode}</code>
        <Button variant="ghost" onClick={copy} aria-label={m.copy_code()}>
          {copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
          {copied ? m.copied() : m.copy()}
        </Button>
      </div>
      <a
        className="ui-button ui-button-primary provider-continue"
        href={challenge.verificationUrl}
        target="_blank"
        rel="noopener noreferrer"
      >
        {m.continue_to_openai()} <ArrowUpRight aria-hidden="true" />
        <span className="sr-only"> {m.opens_a_new_tab()}</span>
      </a>
      <p className="provider-expiry">
        {m.expires_at()}{" "}
        <time dateTime={challenge.expiresAt}>
          {new Date(challenge.expiresAt).toLocaleTimeString(locale)}
        </time>
      </p>
      <p className="provider-copy-status" role="status" aria-live="polite">
        {message && translateMessage(message)}
      </p>
    </div>
  )
}
