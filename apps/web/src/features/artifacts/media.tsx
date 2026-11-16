import { useEffect, useMemo, useState, type ReactNode } from "react"

import { useTheme } from "../../lib/theme/theme"
import { previewHtml } from "./html-document"
import "./media.css"

export type MediaKind =
  | "image"
  | "svg"
  | "video"
  | "pdf"
  | "text"
  | "html"
  | "file"

/** Isolation level of the area a renderer draws into. */
export type MediaSurface =
  | "inline"
  | "isolated-image"
  | "isolated-frame"
  | "download"

/** Largest text, in bytes, shown in the page; larger files are downloaded. */
export const maxMediaTextBytes = 262_144

export type MediaSource = {
  kind: MediaKind
  name: string
  size: number
  contentUrl: string
  downloadUrl: string
  /** Loads at most `limit` bytes of text; the consumer owns network and session. */
  loadText?: (limit: number, signal: AbortSignal) => Promise<string>
}

export const mediaLabels = {
  download: "Download",
  loading: "Loading…",
  error: "This file could not be displayed.",
  tooLarge: "This file is too large to display here.",
}

export type MediaLabels = Record<keyof typeof mediaLabels, string>

export type MediaRenderer = {
  kind: MediaKind
  surface: MediaSurface
  render: (source: MediaSource, labels: MediaLabels) => ReactNode
}

function DownloadLink({
  source,
  labels,
}: {
  source: MediaSource
  labels: MediaLabels
}) {
  return (
    <a
      className="media-download"
      href={source.downloadUrl}
      download={source.name}
      rel="noreferrer"
    >
      {labels.download} · {source.name}
    </a>
  )
}

type TextState =
  | { status: "loading" }
  | { status: "error" }
  | { status: "ready"; text: string }

function useBoundedText(source: MediaSource): TextState | "too-large" {
  const { loadText, size, contentUrl } = source
  const tooLarge = size > maxMediaTextBytes || !loadText
  const [state, setState] = useState<{ key: string; value: TextState } | null>(
    null
  )

  useEffect(() => {
    if (tooLarge || !loadText) return
    const controller = new AbortController()
    loadText(maxMediaTextBytes, controller.signal).then(
      (text) => {
        if (!controller.signal.aborted)
          setState({ key: contentUrl, value: { status: "ready", text } })
      },
      () => {
        if (!controller.signal.aborted)
          setState({ key: contentUrl, value: { status: "error" } })
      }
    )
    return () => controller.abort()
  }, [loadText, tooLarge, contentUrl])

  if (tooLarge) return "too-large"
  return state?.key === contentUrl ? state.value : { status: "loading" }
}

function Status({ children }: { children: ReactNode }) {
  return (
    <p className="media-status" role="status">
      {children}
    </p>
  )
}

function TextMedia({
  source,
  labels,
}: {
  source: MediaSource
  labels: MediaLabels
}) {
  const state = useBoundedText(source)
  if (state === "too-large")
    return (
      <>
        <Status>{labels.tooLarge}</Status>
        <DownloadLink source={source} labels={labels} />
      </>
    )
  if (state.status === "loading") return <Status>{labels.loading}</Status>
  if (state.status === "error")
    return (
      <>
        <Status>{labels.error}</Status>
        <DownloadLink source={source} labels={labels} />
      </>
    )
  return (
    <pre className="media-text" tabIndex={0} aria-label={source.name}>
      <code>{state.text}</code>
    </pre>
  )
}

function HtmlFrame({ html, title }: { html: string; title: string }) {
  return (
    <div className="media-frame">
      <iframe
        title={title}
        sandbox=""
        referrerPolicy="no-referrer"
        srcDoc={html}
      />
    </div>
  )
}

function HtmlMedia({
  source,
  labels,
}: {
  source: MediaSource
  labels: MediaLabels
}) {
  const theme = useTheme()
  const state = useBoundedText(source)
  const text =
    typeof state === "object" && state.status === "ready" ? state.text : null
  const html = useMemo(
    () => (text === null ? null : previewHtml(text, theme)),
    [text, theme]
  )
  if (state !== "too-large" && state.status === "loading")
    return <Status>{labels.loading}</Status>
  if (html) return <HtmlFrame html={html} title={source.name} />
  return (
    <>
      <Status>{state === "too-large" ? labels.tooLarge : labels.error}</Status>
      <DownloadLink source={source} labels={labels} />
    </>
  )
}

const renderers: Record<MediaKind, MediaRenderer> = {
  image: {
    kind: "image",
    surface: "isolated-image",
    render: (source) => (
      <img
        className="media-image"
        src={source.contentUrl}
        alt={source.name}
        loading="lazy"
        referrerPolicy="no-referrer"
      />
    ),
  },
  // SVG is only ever an image resource: its markup is never put in the page.
  svg: {
    kind: "svg",
    surface: "isolated-image",
    render: (source) => (
      <img
        className="media-image"
        src={source.contentUrl}
        alt={source.name}
        loading="lazy"
        referrerPolicy="no-referrer"
      />
    ),
  },
  video: {
    kind: "video",
    surface: "isolated-frame",
    render: (source) => (
      <video
        className="media-video"
        controls
        preload="metadata"
        src={source.contentUrl}
        aria-label={source.name}
      />
    ),
  },
  pdf: {
    kind: "pdf",
    surface: "isolated-frame",
    render: (source) => (
      <iframe
        className="media-pdf"
        title={source.name}
        src={source.contentUrl}
        referrerPolicy="no-referrer"
      />
    ),
  },
  text: {
    kind: "text",
    surface: "inline",
    render: (source, labels) => <TextMedia source={source} labels={labels} />,
  },
  html: {
    kind: "html",
    surface: "isolated-frame",
    render: (source, labels) => <HtmlMedia source={source} labels={labels} />,
  },
  file: {
    kind: "file",
    surface: "download",
    render: (source, labels) => (
      <DownloadLink source={source} labels={labels} />
    ),
  },
}

export const mediaRegistry = {
  resolve(kind: MediaKind): MediaRenderer {
    return renderers[kind] ?? renderers.file
  },
}

export function MediaView({
  source,
  labels,
}: {
  source: MediaSource
  labels?: Partial<MediaLabels>
}) {
  const renderer = mediaRegistry.resolve(source.kind)
  return (
    <figure
      className="media"
      data-kind={renderer.kind}
      data-surface={renderer.surface}
    >
      {/* Changing the content resets loading state. */}
      <div key={source.contentUrl}>
        {renderer.render(source, { ...mediaLabels, ...labels })}
      </div>
    </figure>
  )
}
