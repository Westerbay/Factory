import * as m from "@workspace/i18n/messages"
import { useLocale } from "../../lib/i18n"
import {
  Children,
  createContext,
  isValidElement,
  lazy,
  memo,
  Suspense,
  useContext,
  useId,
  useState,
  type ReactNode,
} from "react"
import { Eye, EyeOff } from "lucide-react"
import Markdown, { type Components } from "react-markdown"
import remarkGfm from "remark-gfm"

import { Button } from "../../components/ui/button"
import { useArtifactPreviewLabels } from "./artifact-labels"
import "./message-markdown.css"

const PreviewsEnabled = createContext(true)
const ArtifactPreview = lazy(async () => {
  const preview = await import("../artifacts/artifact-preview")
  return { default: preview.ArtifactPreview }
})

function markdownUrl(value: string): string | undefined {
  if (!/^(https?:\/\/|mailto:)/i.test(value)) return undefined
  try {
    const url = new URL(value)
    if (url.username || url.password) return undefined
    return url.href
  } catch {
    return undefined
  }
}

const components: Components = {
  // Message headings sit below the workspace's page heading.
  h1: ({ children }) => <h2>{children}</h2>,
  h2: ({ children }) => <h3>{children}</h3>,
  h3: ({ children }) => <h4>{children}</h4>,
  h4: ({ children }) => <h5>{children}</h5>,
  h5: ({ children }) => <h6>{children}</h6>,
  h6: ({ children }) => <h6>{children}</h6>,
  // Footnotes remain readable text, without cross-message DOM IDs or links.
  li: ({ children, className, value }) => (
    <li className={className} value={value}>
      {children}
    </li>
  ),
  a: ({ href, children, title }) =>
    href ? (
      <a href={href} title={title} target="_blank" rel="noopener noreferrer">
        {children}
      </a>
    ) : (
      <span>{children}</span>
    ),
  // Model output must not cause automatic requests to third-party URLs.
  img: ({ alt }) => (
    <span className="message-markdown-image">
      {alt ? `Image: ${alt}` : "Image"}
    </span>
  ),
  pre: CodeBlock,
  table: AccessibleTable,
}

function CodeBlock({ children }: { children?: ReactNode }) {
  useLocale()
  const labels = useArtifactPreviewLabels()
  const previews = useContext(PreviewsEnabled)
  const previewId = useId()
  const [openedSource, setOpenedSource] = useState<string | null>(null)
  const child = Children.toArray(children)[0]
  const code = isValidElement<{
    className?: string
    children?: ReactNode
  }>(child)
    ? child.props
    : undefined
  const language = code?.className?.toLowerCase()
  const kind =
    language === "language-html"
      ? "html"
      : language === "language-mermaid"
        ? "mermaid"
        : null
  const source = typeof code?.children === "string" ? code.children : null
  const sourceBlock = (
    <pre tabIndex={0} aria-label={m.code_block()}>
      {children}
    </pre>
  )
  if (!previews || !kind || !source?.trim()) return sourceBlock

  const open = openedSource === source
  const title =
    kind === "mermaid" ? m.artifact_diagram() : m.artifact_html_document()
  return (
    <div className="message-markdown-code-preview">
      <div className="message-markdown-code-toolbar">
        <span>{kind === "mermaid" ? "Mermaid" : "HTML"}</span>
        <Button
          variant="ghost"
          aria-expanded={open}
          aria-controls={previewId}
          onClick={() => setOpenedSource(open ? null : source)}
        >
          {open ? (
            <EyeOff size={14} aria-hidden />
          ) : (
            <Eye size={14} aria-hidden />
          )}
          {open ? m.artifact_hide_preview() : m.artifact_preview()}
        </Button>
      </div>
      {sourceBlock}
      <div id={previewId} hidden={!open}>
        {open && (
          <Suspense fallback={<p role="status">{m.loading()}</p>}>
            <ArtifactPreview
              artifact={{ kind, title, source }}
              labels={labels}
              compact
            />
          </Suspense>
        )}
      </div>
    </div>
  )
}

function AccessibleTable({ children }: { children?: ReactNode }) {
  useLocale()
  return (
    <div
      className="message-markdown-table"
      role="region"
      aria-label={m.table()}
      tabIndex={0}
    >
      <table>{children}</table>
    </div>
  )
}

/** Render completed and in-progress agent text without executing embedded content. */
export const MessageMarkdown = memo(function MessageMarkdown({
  children,
  previews = true,
}: {
  children: string
  previews?: boolean
}) {
  return (
    <PreviewsEnabled value={previews}>
      <div className="message-markdown">
        <Markdown
          remarkPlugins={[remarkGfm]}
          components={components}
          skipHtml
          urlTransform={markdownUrl}
        >
          {children}
        </Markdown>
      </div>
    </PreviewsEnabled>
  )
})
