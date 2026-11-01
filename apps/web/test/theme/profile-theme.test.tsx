import { renderToStaticMarkup } from "react-dom/server"
import { expect, it, vi } from "vitest"

vi.mock("../../src/lib/auth/session-context", () => ({
  useSession: () => ({
    state: {
      status: "active",
      session: { user: { handle: "demo", role: "member" } },
    },
    controller: {},
  }),
}))
vi.mock("../../src/lib/theme/theme", () => ({
  useTheme: () => "light",
  setTheme: vi.fn(),
}))

import { ProfileMenu } from "../../src/components/workspace/profile-menu"

it("keeps independently mounted profile menus in separate radio groups", () => {
  const menu = <ProfileMenu canStop onSettings={() => {}} onStop={() => {}} />
  const html = renderToStaticMarkup(
    <>
      {menu}
      {menu}
    </>
  )
  const radios = [...html.matchAll(/<input[^>]+type="radio"[^>]*>/g)].map(
    (match) => match[0]
  )
  const names = radios.map((radio) => radio.match(/name="([^"]+)"/)?.[1])
  expect(radios).toHaveLength(4)
  expect(names[0]).toBe(names[1])
  expect(names[2]).toBe(names[3])
  expect(names[0]).not.toBe(names[2])
  expect(radios.filter((radio) => radio.includes('checked=""'))).toHaveLength(2)
})
