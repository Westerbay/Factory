import { describe, expect, it, vi } from "vitest"

vi.mock("../../src/lib/theme/theme", () => ({ useTheme: () => "dark" }))

import { diagramScale } from "../../src/features/artifacts/artifact-preview"

describe("diagram preview scale", () => {
  const largeDiagram = { width: 5582, height: 220 }
  const compactViewport = { width: 298, height: 256 }

  it("reaches the natural readable size of a large diagram independently of Fit", () => {
    const fitted = diagramScale(largeDiagram, compactViewport, null)
    expect(fitted).toBeLessThan(0.05)
    expect(largeDiagram.width * fitted).toBeCloseTo(266)
    expect(
      largeDiagram.width * diagramScale(largeDiagram, compactViewport, 1)
    ).toBe(5582)
    expect(16 * diagramScale(largeDiagram, compactViewport, 1)).toBe(16)
    expect(16 * diagramScale(largeDiagram, compactViewport, 3)).toBe(48)
  })

  it("keeps manual zoom unchanged when the viewport expands, while Fit adapts", () => {
    const expandedViewport = { width: 1200, height: 720 }
    expect(diagramScale(largeDiagram, expandedViewport, 1.25)).toBe(
      diagramScale(largeDiagram, compactViewport, 1.25)
    )
    expect(diagramScale(largeDiagram, expandedViewport, null)).toBeGreaterThan(
      diagramScale(largeDiagram, compactViewport, null)
    )
  })

  it("fits both dimensions without enlarging an already small diagram", () => {
    expect(
      diagramScale({ width: 180, height: 100 }, compactViewport, null)
    ).toBe(1)
    const tall = { width: 200, height: 1000 }
    expect(tall.height * diagramScale(tall, compactViewport, null)).toBe(224)
  })
})
