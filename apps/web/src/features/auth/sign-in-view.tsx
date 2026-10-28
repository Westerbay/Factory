import { API_ORIGIN } from "../../lib/api/client"
import "./auth.css"

const reasons = {
  pending:
    "Your access request is waiting for administrator approval. Sign in again after your access is approved.",
  suspended:
    "Your access has been suspended. Contact your administrator before trying again.",
  denied: "GitHub sign-in was cancelled. You can try again when you are ready.",
  unavailable: "Sign-in is temporarily unavailable. Please try again later.",
  "invalid-login":
    "This sign-in attempt expired or could not be verified. Start a new sign-in below.",
} as const

export function signInReason(value: unknown): keyof typeof reasons | undefined {
  return typeof value === "string" && Object.hasOwn(reasons, value)
    ? (value as keyof typeof reasons)
    : undefined
}

export function SignInView({ reason }: { reason?: keyof typeof reasons }) {
  return (
    <section className="auth-page" aria-labelledby="sign-in-title">
      <p className="eyebrow">Your private workspace</p>
      <h1 id="sign-in-title">Welcome to Factory</h1>
      <p className="introduction">
        Sign in with GitHub to access your workspace. An administrator must
        approve your access first.
      </p>
      {reason && (
        <p className="auth-notice" role="status">
          {reasons[reason]}
        </p>
      )}
      <a
        className="primary-action"
        href={`${API_ORIGIN}/api/auth/github/start`}
      >
        Continue with GitHub
      </a>
      <p className="auth-note">
        No password or email registration is required.
      </p>
    </section>
  )
}
