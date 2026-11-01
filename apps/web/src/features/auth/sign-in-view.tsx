import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import { Brand } from "../../components/ui/brand"
import { API_ORIGIN } from "../../lib/api/client"
import "./auth.css"

const reasons = {
  pending:
    "Your access request is waiting for administrator approval. Sign in again after your access is approved.",
  suspended:
    "Your access has been suspended. Contact your administrator before trying again.",
  denied: "GitHub sign-in was cancelled. You can try again when you are ready.",
  unavailable: "Sign-in is temporarily unavailable. Please try again later.",
  "email-unavailable":
    "GitHub did not provide a verified primary email. Verify your email on GitHub, then try again.",
  "github-forbidden":
    "GitHub denied access. Check your account or organization permissions, then try again.",
  "github-rate-limited":
    "GitHub is temporarily rate limiting requests. Try signing in again later.",
  "invalid-login":
    "This sign-in attempt expired or could not be verified. Start a new sign-in below.",
} as const

export function signInReason(value: unknown): keyof typeof reasons | undefined {
  return typeof value === "string" && Object.hasOwn(reasons, value)
    ? (value as keyof typeof reasons)
    : undefined
}

export function SignInView({ reason }: { reason?: keyof typeof reasons }) {
  useLocale()
  return (
    <section className="auth-page" aria-labelledby="sign-in-title">
      <Brand />
      <div className="auth-content">
        <h1 id="sign-in-title">{m.sign_in_to_factory()}</h1>
        <p>
          {m.github_access_lets_factory_read_and_write_your_private_repositori()}
        </p>
        {reason && (
          <p className="auth-notice" role="status">
            {translateMessage(reasons[reason])}
          </p>
        )}
        <a
          className="ui-button ui-button-primary auth-sign-in"
          href={`${API_ORIGIN}/api/auth/github/start`}
        >
          <img src="/brand/github.svg" width="20" height="20" alt="" />
          {m.continue_with_github()}
        </a>
      </div>
    </section>
  )
}
