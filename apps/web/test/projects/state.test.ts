import { describe, expect, it } from "vitest"

import {
  githubCallbackMessage,
  mergeProjectPage,
  projectWarnings,
} from "../../src/features/projects/state"
import type { Project } from "../../src/features/projects/api"

const first: Project = {
  id: "first",
  fullName: "owner/first",
  state: "ready",
  renamedSinceClone: false,
}
describe("loaded projects", () => {
  it("drops an old boundary project that shifted off the refreshed pages", () => {
    const oldPages = Array.from({ length: 200 }, (_, index) => ({
      ...first,
      id: String(index),
      fullName: `owner/project-${index}`,
    }))
    const shifted = {
      ...oldPages[199]!,
      fullName: "owner/renamed",
      renamedSinceClone: true,
    }
    const current = [{ ...first, id: "new" }, ...oldPages.slice(0, 199)]
    let visible = mergeProjectPage(oldPages, current.slice(0, 100), true)
    visible = mergeProjectPage(visible, current.slice(100, 200), false)
    expect(visible).toHaveLength(200)
    expect(visible.some((project) => project.id === shifted.id)).toBe(false)
    expect(mergeProjectPage(visible, [shifted], false).at(-1)).toEqual(shifted)
  })
})

it("reports an invalid authorization without assuming an identity mismatch", () => {
  expect(githubCallbackMessage("authorization-invalid")).toBe(
    "GitHub authorization could not be confirmed. Try connecting again."
  )
  expect(githubCallbackMessage("identity-mismatch")).toContain("did not match")
})

it("preserves only confirmed writes while revalidating paginated results", () => {
  const imported = { ...first, id: "imported", state: "importing" as const }
  const stale = { ...first, id: "stale" }
  const refreshed = mergeProjectPage([stale, imported], [first], true, [
    imported,
  ])
  expect(refreshed).toEqual([first, imported])
  const ready = { ...imported, state: "ready" as const }
  expect(mergeProjectPage(refreshed, [ready], false)).toEqual([first, ready])
})

it("describes address uncertainty without claiming to inspect a remote", () => {
  expect(projectWarnings("project", "checking", false)).toEqual([])
  expect(projectWarnings("project", "unverified", false)).toEqual([
    "GitHub address not verified; check your Git remote before pushing.",
  ])
  expect(projectWarnings("project", "changed", false)[0]).toContain(
    "or is inaccessible"
  )
  expect(projectWarnings(undefined, "unverified", true)).toEqual([])
})
