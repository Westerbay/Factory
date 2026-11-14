import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it, vi } from "vitest"

vi.mock("../../src/lib/theme/theme", () => ({ useTheme: () => "dark" }))

import {
  MediaView,
  maxMediaTextBytes,
  mediaRegistry,
  type MediaKind,
  type MediaSource,
} from "../../src/features/artifacts/media"

const kinds: MediaKind[] = [
  "image",
  "svg",
  "video",
  "pdf",
  "text",
  "html",
  "file",
]

function source(kind: MediaKind, extra: Partial<MediaSource> = {}) {
  return {
    kind,
    name: "report",
    size: 10,
    contentUrl: "/content/1",
    downloadUrl: "/download/1",
    loadText: async () => "hello",
    ...extra,
  } satisfies MediaSource
}

function render(kind: MediaKind, extra: Partial<MediaSource> = {}) {
  return renderToStaticMarkup(<MediaView source={source(kind, extra)} />)
}

describe("media registry", () => {
  it("maps every kind to its renderer and surface", () => {
    expect(
      Object.fromEntries(
        kinds.map((k) => [k, mediaRegistry.resolve(k).surface])
      )
    ).toEqual({
      image: "isolated-image",
      svg: "isolated-image",
      video: "isolated-frame",
      pdf: "isolated-frame",
      text: "inline",
      html: "isolated-frame",
      file: "download",
    })
    for (const k of kinds) expect(mediaRegistry.resolve(k).kind).toBe(k)
  })

  it("falls back to a download for an unknown kind", () => {
    expect(mediaRegistry.resolve("other" as MediaKind).kind).toBe("file")
  })
})

describe("media isolation", () => {
  it("renders images and SVG only through <img>", () => {
    for (const kind of ["image", "svg"] as const) {
      const html = render(kind)
      expect(html).toContain('<img class="media-image" src="/content/1"')
      expect(html).not.toMatch(/<svg|<object|<embed|<iframe/)
    }
  })

  it("renders video with controls and metadata preloading", () => {
    const html = render("video")
    expect(html).toContain("<video")
    expect(html).toContain("controls")
    expect(html).toContain('preload="metadata"')
  })

  it("points the PDF frame at the content URL", () => {
    expect(render("pdf")).toMatch(/<iframe[^>]*src="\/content\/1"/)
  })

  it("offers a download link for generic files", () => {
    const html = render("file")
    expect(html).toContain('href="/download/1"')
    expect(html).toContain("download=")
  })

  it("never loads text or HTML above the bound, offering a download", () => {
    const loadText = vi.fn(async () => "x")
    for (const kind of ["text", "html"] as const) {
      const html = render(kind, { size: maxMediaTextBytes + 1, loadText })
      expect(html).toContain('href="/download/1"')
      expect(html).not.toContain("<pre")
      expect(html).not.toContain("<iframe")
    }
    expect(loadText).not.toHaveBeenCalled()
  })

  it("waits for text before showing it and never injects markup", () => {
    for (const kind of ["text", "html"] as const) {
      const html = render(kind)
      expect(html).toContain("Loading…")
      expect(html).not.toContain("<iframe")
    }
  })

  it("does not touch the network by itself", () => {
    const fetch = vi.fn()
    vi.stubGlobal("fetch", fetch)
    for (const kind of kinds) render(kind)
    expect(fetch).not.toHaveBeenCalled()
    vi.unstubAllGlobals()
  })
})
