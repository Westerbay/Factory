import { useEffect, useId, useRef, useState } from "react"
import { ArrowUpRight, Check, Copy } from "lucide-react"

import { Button } from "../../components/ui/button"
import type { LoginChallenge } from "./api"

export function DeviceCode({ challenge }: { challenge: LoginChallenge }) {
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
      <p>Enter this code on OpenAI to connect your account.</p>
      <div className="provider-code-row">
        <span className="sr-only" id={codeId}>
          Authorization code
        </span>
        <code aria-labelledby={codeId}>{userCode}</code>
        <Button variant="ghost" onClick={copy} aria-label="Copy code">
          {copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
      <a
        className="ui-button ui-button-primary provider-continue"
        href={challenge.verificationUrl}
        target="_blank"
        rel="noopener noreferrer"
      >
        Continue to OpenAI
        <ArrowUpRight aria-hidden="true" />
        <span className="sr-only"> (opens a new tab)</span>
      </a>
      <p className="provider-expiry">
        Expires at{" "}
        <time dateTime={challenge.expiresAt}>
          {new Date(challenge.expiresAt).toLocaleTimeString()}
        </time>
      </p>
      <p className="provider-copy-status" role="status" aria-live="polite">
        {message}
      </p>
    </div>
  )
}
