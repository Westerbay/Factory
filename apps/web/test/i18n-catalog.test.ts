import { describe, expect, it } from "vitest"

import english from "../../../packages/i18n/messages/en.json"
import french from "../../../packages/i18n/messages/fr.json"

function variables(message: unknown): string[] {
  const text = JSON.stringify(message)
  return [
    ...new Set(
      [...text.matchAll(/\{([a-zA-Z][a-zA-Z0-9_]*)\}/g)].map(
        (match) => match[1]!
      )
    ),
  ].sort()
}

describe("Paraglide message catalogs", () => {
  it("have matching message keys and interpolation variables", () => {
    expect(Object.keys(french).sort()).toEqual(Object.keys(english).sort())
    for (const key of Object.keys(english) as Array<keyof typeof english>)
      expect(variables(french[key]), key).toEqual(variables(english[key]))
  })
})
