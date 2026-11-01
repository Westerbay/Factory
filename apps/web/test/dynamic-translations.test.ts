import { describe, expect, it } from "vitest"

import { setLanguage, translateMessage } from "../src/lib/i18n"
import { connectionProblem } from "../src/features/agent-connections/api"
import { environmentProblem } from "../src/features/environments/api"
import { terminalProblem } from "../src/features/terminal/api"
import {
  githubCallbackMessage,
  projectWarnings,
} from "../src/features/projects/state"
import { questionReceipt } from "../src/features/conversations/state"

describe("French runtime messages", () => {
  it("covers messages produced by environment, provider, terminal and recovery states", () => {
    const messages = [
      ...[
        "volume_missing",
        "provisioning_uncertain",
        "ownership_conflict",
        "policy_mismatch",
        "invalid_configuration",
        "timeout",
      ].map(environmentProblem),
      ...[
        "unavailable",
        "unsupported_method",
        "login_failed",
        "invalid_provider_response",
        "cancelled",
        "expired",
        "timeout",
        "cleanup_uncertain",
      ].map(connectionProblem),
      ...(
        [
          "unauthorized",
          "unavailable",
          "conflict",
          "invalid_ticket",
          "invalid_input",
          "slow_client",
          "cleanup_uncertain",
        ] as const
      ).map(terminalProblem),
      ...[
        "connected",
        "denied",
        "authorization-invalid",
        "identity-mismatch",
        "forbidden",
        "expired",
        "rate-limited",
        "email-unavailable",
        "unavailable",
        "unknown",
      ].map(githubCallbackMessage),
      ...projectWarnings("project", "changed", true),
      ...projectWarnings("project", "unverified", false),
      questionReceipt("uncertain", false).message,
      questionReceipt("claimed", false).message,
    ]
    setLanguage("fr")
    try {
      expect(
        messages.every(
          (message) => message && translateMessage(message) !== message
        )
      ).toBe(true)
    } finally {
      setLanguage("en")
    }
  })
})
