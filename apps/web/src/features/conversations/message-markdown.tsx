import * as m from "@workspace/i18n/messages"
import { useLocale } from "../../lib/i18n"
import { memo, type ReactNode } from "react"
import Markdown, { type Components } from "react-markdown"
import remarkGfm from "remark-gfm"

import "./message-markdown.css"

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
  return (
    <pre tabIndex={0} aria-label={m.code_block()}>
      {children}
    </pre>
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
}: {
  children: string
}) {
  return (
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
  )
})
