import { useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import {
  Code2,
  Maximize2,
  Minus,
  Monitor,
  Plus,
  Scan,
  Smartphone,
} from "lucide-react"

import { Button, IconButton } from "../../components/ui/button"
import { Modal } from "../../components/ui/modal"
import { useTheme } from "../../lib/theme/theme"
import { previewHtml } from "./html-document"
import type { DiagramImage } from "./mermaid-renderer"
import "./artifact-preview.css"

export type PreviewArtifact = {
  kind: "mermaid" | "html"
  title: string
  source: string
}

export const artifactPreviewLabels = {
  source: "Source",
  hideSource: "Hide source",
  preview: "Preview",
  diagramLoading: "Rendering diagram…",
  diagramError: "This diagram could not be displayed.",
  htmlError: "This document could not be displayed.",
  fit: "Fit diagram",
  fitted: "Fitted",
  zoomOut: "Zoom out",
  zoomIn: "Zoom in",
  expand: "Expand diagram",
  close: "Close",
  desktop: "Desktop width",
  mobile: "Mobile width",
  scrollable: "Scrollable diagram",
}

export type ArtifactPreviewLabels = Record<
  keyof typeof artifactPreviewLabels,
  string
>

type PreviewProps = {
  title: string
  source: string
  labels: ArtifactPreviewLabels
  compact?: boolean
}

function Source({ source, title }: { source: string; title: string }) {
  return (
    <pre className="artifact-preview-source" tabIndex={0} aria-label={title}>
      <code>{source}</code>
    </pre>
  )
}

/** Numeric zoom is the SVG's natural scale; null fits the available viewport. */
export function diagramScale(
  image: Pick<DiagramImage, "width" | "height">,
  viewport: { width: number; height: number },
  zoom: number | null
): number {
  return (
    zoom ??
    Math.min(
      1,
      Math.max(1, viewport.width - 32) / image.width,
      Math.max(1, viewport.height - 32) / image.height
    )
  )
}

function DiagramCanvas({
  image,
  title,
  labels,
  zoom,
  onFit,
}: {
  image: DiagramImage
  title: string
  labels: ArtifactPreviewLabels
  zoom: number | null
  onFit: (scale: number) => void
}) {
  const viewport = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ width: 0, height: 0 })

  useEffect(() => {
    const element = viewport.current
    if (!element) return
    const observer = new ResizeObserver(([entry]) => {
      if (entry) {
        setSize({
          width: entry.contentRect.width,
          height: entry.contentRect.height,
        })
      }
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const fit = diagramScale(image, size, null)
  useEffect(() => {
    if (size.width > 0 && size.height > 0) onFit(fit)
  }, [fit, onFit, size.width, size.height])
  const scale = diagramScale(image, size, zoom)

  return (
    <div
      ref={viewport}
      className="artifact-preview-viewport"
      tabIndex={0}
      aria-label={`${title} · ${labels.scrollable}`}
    >
      <div className="artifact-preview-centering">
        <img
          src={image.src}
          alt={title}
          style={{ width: image.width * scale, height: image.height * scale }}
        />
      </div>
    </div>
  )
}

function DiagramPreview({ title, source, labels, compact }: PreviewProps) {
  const theme = useTheme()
  const [render, setRender] = useState<{
    theme: typeof theme
    image: DiagramImage | null
  } | null>(null)
  const [showSource, setShowSource] = useState(false)
  const [expanded, setExpanded] = useState(false)
  const [zoom, setZoom] = useState<number | null>(null)
  const [inlineFit, setInlineFit] = useState(1)
  const [expandedFit, setExpandedFit] = useState(1)
  const opener = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    const controller = new AbortController()
    async function load() {
      try {
        const { renderMermaid } = await import("./mermaid-renderer")
        const image = await renderMermaid(
          source,
          theme === "dark",
          controller.signal
        )
        if (!controller.signal.aborted) setRender({ theme, image })
      } catch {
        if (!controller.signal.aborted) setRender({ theme, image: null })
      }
    }
    void load()
    return () => controller.abort()
  }, [source, theme])

  const current = render?.theme === theme ? render : null
  const image = current?.image
  const failed = current !== null && !image
  const fit = expanded ? expandedFit : inlineFit
  const minimumZoom = Math.min(0.25, fit)
  const currentZoom = zoom ?? fit

  function changeZoom(delta: number) {
    setZoom((currentZoom) =>
      Math.max(minimumZoom, Math.min(3, (currentZoom ?? fit) + delta))
    )
  }

  function controls(end: ReactNode) {
    return (
      <div className="artifact-preview-actions">
        <IconButton
          label={labels.zoomOut}
          disabled={!image || currentZoom <= minimumZoom}
          onClick={() => changeZoom(-0.25)}
        >
          <Minus size={14} aria-hidden />
        </IconButton>
        <span className="artifact-preview-zoom" aria-live="polite">
          {zoom === null ? labels.fitted : `${Math.round(zoom * 100)}%`}
        </span>
        <IconButton
          label={labels.zoomIn}
          disabled={!image || currentZoom >= 3}
          onClick={() => changeZoom(0.25)}
        >
          <Plus size={14} aria-hidden />
        </IconButton>
        <Button
          variant="ghost"
          className="artifact-preview-native-size"
          disabled={!image}
          aria-pressed={zoom === 1}
          onClick={() => setZoom(1)}
        >
          100%
        </Button>
        <IconButton
          label={labels.fit}
          disabled={!image}
          aria-pressed={zoom === null}
          onClick={() => setZoom(null)}
        >
          <Scan size={14} aria-hidden />
        </IconButton>
        {end}
      </div>
    )
  }

  return (
    <section
      aria-label={title}
      className="artifact-preview"
      data-compact={compact || undefined}
    >
      <div className="artifact-preview-toolbar">
        <span className="artifact-preview-title">{title}</span>
        {controls(
          <IconButton
            ref={opener}
            label={labels.expand}
            disabled={!image}
            onClick={() => setExpanded(true)}
          >
            <Maximize2 size={14} aria-hidden />
          </IconButton>
        )}
      </div>
      {image && (
        <DiagramCanvas
          image={image}
          title={title}
          labels={labels}
          zoom={zoom}
          onFit={setInlineFit}
        />
      )}
      {!current && (
        <p
          className="artifact-preview-status artifact-preview-viewport artifact-preview-loading"
          role="status"
        >
          {labels.diagramLoading}
        </p>
      )}
      {failed && (
        <p className="artifact-preview-status" role="status">
          {labels.diagramError}
        </p>
      )}
      <div className="artifact-preview-caption">
        <span>Mermaid</span>
        {!failed && (
          <Button
            variant="ghost"
            aria-expanded={showSource}
            onClick={() => setShowSource((current) => !current)}
          >
            <Code2 size={14} aria-hidden />
            {showSource ? labels.hideSource : labels.source}
          </Button>
        )}
      </div>
      {(showSource || failed) && (
        <Source source={source} title={labels.source} />
      )}
      <Modal
        title={title}
        open={expanded}
        onClose={() => setExpanded(false)}
        returnFocusRef={opener}
        className="artifact-preview-modal"
      >
        {controls(
          <Button onClick={() => setExpanded(false)}>{labels.close}</Button>
        )}
        {expanded && image && (
          <DiagramCanvas
            image={image}
            title={title}
            labels={labels}
            zoom={zoom}
            onFit={setExpandedFit}
          />
        )}
      </Modal>
    </section>
  )
}

function HtmlPreview({ title, source, labels }: PreviewProps) {
  const theme = useTheme()
  const [showSource, setShowSource] = useState(false)
  const [mobile, setMobile] = useState(false)
  const html = useMemo(() => previewHtml(source, theme), [source, theme])

  return (
    <section aria-label={title} className="artifact-preview">
      <div className="artifact-preview-toolbar">
        <span className="artifact-preview-title">{title}</span>
        <div className="artifact-preview-actions">
          <Button
            variant="ghost"
            aria-pressed={!showSource}
            onClick={() => setShowSource(false)}
          >
            {labels.preview}
          </Button>
          <Button
            variant="ghost"
            aria-pressed={showSource}
            onClick={() => setShowSource(true)}
          >
            {labels.source}
          </Button>
          {!showSource && html && (
            <>
              <IconButton
                label={labels.desktop}
                aria-pressed={!mobile}
                onClick={() => setMobile(false)}
              >
                <Monitor size={14} aria-hidden />
              </IconButton>
              <IconButton
                label={labels.mobile}
                aria-pressed={mobile}
                onClick={() => setMobile(true)}
              >
                <Smartphone size={14} aria-hidden />
              </IconButton>
            </>
          )}
        </div>
      </div>
      {!html && (
        <p className="artifact-preview-status" role="status">
          {labels.htmlError}
        </p>
      )}
      {showSource || !html ? (
        <Source source={source} title={labels.source} />
      ) : (
        <div
          className="artifact-preview-html"
          data-mobile={mobile || undefined}
        >
          <iframe
            title={title}
            sandbox=""
            referrerPolicy="no-referrer"
            srcDoc={html}
          />
        </div>
      )}
    </section>
  )
}

export function ArtifactPreview({
  artifact,
  labels,
  compact,
}: {
  artifact: PreviewArtifact
  labels?: Partial<ArtifactPreviewLabels>
  compact?: boolean
}) {
  const props = {
    title: artifact.title,
    source: artifact.source,
    labels: { ...artifactPreviewLabels, ...labels },
    compact,
  }
  // Changing the revision resets source toggles and abandons an in-flight render.
  return artifact.kind === "mermaid" ? (
    <DiagramPreview key={artifact.source} {...props} />
  ) : (
    <HtmlPreview key={artifact.source} {...props} />
  )
}
