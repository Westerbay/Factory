import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it } from "vitest"

import { MessageMarkdown } from "../../src/features/conversations/message-markdown"

function render(content: string, previews = true) {
  return renderToStaticMarkup(
    <MessageMarkdown previews={previews}>{content}</MessageMarkdown>
  )
}

describe("agent message Markdown", () => {
  it("renders headings, paragraphs, emphasis, lists and quotes as semantic content", () => {
    const html = render(
      "# Result\n\n**Ready** with *emphasis* and `code`.\n\n- One\n- Two\n\n1. First\n2. Second\n\n> Note"
    )
    expect(html).toContain("<h2>Result</h2>")
    expect(html).toContain("<strong>Ready</strong>")
    expect(html).toContain("<em>emphasis</em>")
    expect(html).toContain("<code>code</code>")
    expect(html).toContain("<ul>")
    expect(html).toContain("<ol>")
    expect(html).toContain("<blockquote>")
  })

  it("renders GFM tables, strikethrough and disabled task items", () => {
    const html = render(
      "| Name | State |\n| --- | --- |\n| Test | ~~Pending~~ |\n\n- [x] Complete\n- [ ] Next"
    )
    expect(html).toContain(
      '<div class="message-markdown-table" role="region" aria-label="Table" tabindex="0">'
    )
    expect(html).toContain("<table>")
    expect(html).toContain("<th>Name</th>")
    expect(html).toContain("<del>Pending</del>")
    expect(html).toMatch(/<input type="checkbox" disabled="" checked=""\/>/)
    expect(html).toMatch(/<input type="checkbox" disabled=""\/>/)
  })

  it("keeps fenced code inert and preserves its whitespace", () => {
    const html = render("```html\n<script>alert('x')</script>\n  indented\n```")
    expect(html).toContain('<pre tabindex="0" aria-label="Code block">')
    expect(html).toContain('<code class="language-html">')
    expect(html).toContain(
      "&lt;script&gt;alert(&#x27;x&#x27;)&lt;/script&gt;\n  indented\n"
    )
    expect(html).not.toContain("<script>")
  })

  it("renders incomplete streamed fences and later completed syntax", () => {
    const prefix = "A **result**:\n\n```ts\nconst answer ="
    expect(render(prefix)).toContain("const answer =\n</code></pre>")
    expect(render(prefix + " 42\n```\n\nDone.")).toContain("<p>Done.</p>")
    expect(render("A [partially written link](https://exam")).toContain(
      "partially written link"
    )
  })

  it.each(["html", "mermaid"])(
    "offers an opt-in preview for %s fences without mounting the artifact",
    (language) => {
      const source =
        language === "html"
          ? '<img src="https://example.test/private">'
          : "graph LR\nA-->B"
      const html = render(`\`\`\`${language}\n${source}\n\`\`\``)
      expect(html).toContain('class="message-markdown-code-toolbar"')
      expect(html).toContain('aria-expanded="false"')
      expect(html).toContain("Preview</button>")
      expect(html).toContain(`<code class="language-${language}">`)
      expect(html).not.toContain('class="artifact-preview"')
      expect(html).not.toMatch(/<(iframe|img)\b/)
    }
  )

  it("keeps preview actions unavailable while an assistant response streams", () => {
    const html = render(
      "```html\n<h1>Pending</h1>\n```\n\n```mermaid\ngraph LR\nA-->B",
      false
    )
    expect(html).toContain('class="language-html"')
    expect(html).toContain('class="language-mermaid"')
    expect(html).not.toContain("<button")
    expect(html).not.toContain("artifact-preview")
  })

  it.each(["javascript", "xml", "svg", "html-example", ""])(
    "does not offer a preview for %s code",
    (language) => {
      const html = render(`\`\`\`${language}\n<h1>Source only</h1>\n\`\`\``)
      expect(html).toContain("&lt;h1&gt;Source only&lt;/h1&gt;")
      expect(html).not.toContain("<button")
    }
  )

  it("does not offer a preview for an empty supported code fence", () => {
    expect(render("```html\n\n``` ")).not.toContain("<button")
  })

  it("opens explicit web and mail links without an opener or referrer", () => {
    const html = render(
      "[Docs](https://example.test/docs) [HTTP](http://example.test/) [Mail](mailto:hello@example.test)"
    )
    expect(html).toContain(
      'href="https://example.test/docs" target="_blank" rel="noopener noreferrer"'
    )
    expect(html).toContain('href="http://example.test/"')
    expect(html).toContain('href="mailto:hello@example.test"')
  })

  it.each([
    "javascript:alert(1)",
    "JaVaScRiPt:alert(1)",
    "data:text/html;base64,PHNjcmlwdD4=",
    "file:///etc/passwd",
    "//example.test/private",
    "/api/auth/logout",
    "https://user:secret@example.test/",
  ])("keeps disallowed destination %s inert", (url) => {
    const html = render(`[Open](<${url}>)`)
    expect(html).not.toContain("href=")
    expect(html).toContain("Open")
  })

  it("does not render raw HTML or load image URLs", () => {
    const html = render(
      '<script>alert(1)</script>\n\n<iframe src="https://example.test/frame"></iframe>\n\n<img src="https://example.test/raw" onerror="alert(1)">\n\n![Diagram](https://example.test/tracker?private=message)'
    )
    expect(html).not.toMatch(/<(script|iframe|img)\b/)
    expect(html).not.toContain("src=")
    expect(html).not.toContain("example.test")
    expect(html).toContain("Image: Diagram")
  })

  it("keeps footnotes readable without cross-message IDs or dead fragment links", () => {
    const html = render("A note[^1].\n\n[^1]: A reference.")
    expect(html).toContain("A reference.")
    expect(html).not.toContain('id="')
    expect(html).not.toContain("aria-describedby")
    expect(html).not.toContain("href=")
  })
})
