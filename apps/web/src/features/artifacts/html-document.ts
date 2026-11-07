import DOMPurify from "dompurify"

const maxHtmlLength = 262_144
const policy =
  "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"

export function previewHtml(source: string, theme: "light" | "dark") {
  if (source.length > maxHtmlLength || !DOMPurify.isSupported) return null

  // Parse in an inert template so raw markup is never loaded as a live document.
  const template = document.createElement("template")
  template.innerHTML = source
  const markup = DOMPurify.sanitize(template.innerHTML, {
    FORCE_BODY: true,
    FORBID_TAGS: [
      "base",
      "button",
      "embed",
      "fencedframe",
      "form",
      "frame",
      "frameset",
      "iframe",
      "input",
      "link",
      "meta",
      "object",
      "portal",
      "select",
      "template",
      "textarea",
    ],
    FORBID_ATTR: [
      "action",
      "autofocus",
      "background",
      "contenteditable",
      "formaction",
      "href",
      "ping",
      "poster",
      "src",
      "srcset",
      "target",
      "xlink:href",
    ],
  })
  return `<!doctype html><html><head><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="${policy}">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>:root { color-scheme: ${theme}; } body { font-family: system-ui; }</style>
    </head><body>${markup}</body></html>`
}
