import { expect, it } from "vitest"

import type { Model } from "../../src/features/conversations/api"
import { presentModels } from "../../src/features/conversations/model-presentation"

it("shows resolved Claude versions and removes only equivalent default aliases", () => {
  const models: Model[] = [
    {
      id: "factory:claude:default",
      name: "Claude · Default (recommended)",
      resolvedModel: "claude-opus-5-5",
      effortLevels: ["low", "high"],
      defaultEffort: null,
    },
    {
      id: "factory:claude:opus",
      name: "Claude · Opus",
      resolvedModel: "claude-opus-5-5",
      effortLevels: ["low", "high"],
      defaultEffort: null,
    },
    {
      id: "factory:claude:sonnet",
      name: "Claude · Sonnet",
      resolvedModel: "claude-sonnet-4-6[1m]",
      effortLevels: [],
      defaultEffort: null,
    },
    {
      id: "factory:claude:haiku",
      name: "Claude · Haiku",
      resolvedModel: "claude-haiku-4-5-20251001",
      effortLevels: [],
      defaultEffort: null,
    },
    {
      id: "factory:codex:gpt-6.1-sol",
      name: "Codex · GPT-6.1-Sol",
      resolvedModel: "gpt-6.1-sol",
      effortLevels: [],
      defaultEffort: null,
    },
  ]
  const options = presentModels(models)
  expect(options.map(({ name }) => name)).toEqual([
    "Claude - Opus 5.5",
    "Claude - Sonnet 4.6 (1M context)",
    "Claude - Haiku 4.5",
    "Codex - GPT-6.1-Sol",
  ])
  expect(options[0]?.id).toBe("factory:claude:opus")
  expect(options[0]?.effortLevels).toEqual(["low", "high"])
})

it("keeps a unique default and leaves unknown resolved identifiers out of the label", () => {
  const options = presentModels([
    {
      id: "factory:claude:default",
      name: "Claude · Default (recommended)",
      resolvedModel: "custom-model",
      effortLevels: ["low"],
      defaultEffort: null,
    },
    {
      id: "factory:claude:opus",
      name: "Claude · Opus",
      resolvedModel: "custom-model",
      effortLevels: ["high"],
      defaultEffort: null,
    },
  ])
  expect(options.map(({ name }) => name)).toEqual([
    "Claude - Default",
    "Claude - Opus",
  ])
})
