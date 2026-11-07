import bundleUrl from "@mermaid-js/tiny/dist/mermaid.tiny.js?url&no-inline"

const maxSourceLength = 32_768
const maxSvgLength = 1_048_576
const renderTimeoutMs = 10_000
const diagramHeader =
  /^(?:flowchart|graph|classDiagram|sequenceDiagram|stateDiagram(?:-v2)?|erDiagram)\b/
const mermaidDirective = /%%\s*\{|^[ \t]*---(?:[ \t]|\r?$)/m

const frameBootstrap = `
  window.addEventListener("message", async function render(event) {
    if (event.source !== window.parent || event.data?.type !== "factory:render-diagram") return;
    window.removeEventListener("message", render);
    const reply = (result) =>
      window.parent.postMessage({type: "factory:rendered-diagram", result}, event.origin);
    try {
      const { source, themeVariables } = event.data;
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: "base",
        look: "classic",
        themeVariables,
        htmlLabels: false,
        suppressErrorRendering: true,
        flowchart: { curve: "linear", padding: 16 },
        class: { hideEmptyMembersBox: true },
        sequence: { mirrorActors: false },
      });
      const { svg } = await mermaid.render("factory-preview", source);
      const image = new DOMParser().parseFromString(svg, "image/svg+xml").documentElement;
      const [, , width, height] = image.getAttribute("viewBox")?.trim().split(/\\s+/).map(Number) ?? [];
      if (image.localName !== "svg" || !Number.isFinite(width) || width <= 0 ||
          !Number.isFinite(height) || height <= 0) throw new Error("Invalid SVG");
      image.setAttribute("width", width);
      image.setAttribute("height", height);
      reply({ svg: new XMLSerializer().serializeToString(image), width, height });
    } catch {
      reply(null);
    }
  });
`

export type DiagramImage = {
  src: string
  width: number
  height: number
}

function imageFromReply(result: unknown): DiagramImage | null {
  if (!result || typeof result !== "object") return null
  const { svg, width, height } = result as Record<string, unknown>
  if (
    typeof svg !== "string" ||
    svg.length > maxSvgLength ||
    typeof width !== "number" ||
    !Number.isFinite(width) ||
    width <= 0 ||
    typeof height !== "number" ||
    !Number.isFinite(height) ||
    height <= 0
  ) {
    return null
  }
  // Loading the SVG as an image keeps its markup out of Factory's document.
  return { src: `data:image/svg+xml,${encodeURIComponent(svg)}`, width, height }
}

function rendererDocument() {
  const nonce = crypto.randomUUID()
  const url = new URL(bundleUrl, window.location.href)
  const safeUrl = url.href.replaceAll("&", "&amp;").replaceAll('"', "&quot;")
  const csp = `default-src 'none'; script-src 'nonce-${nonce}'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'`
  return `<!doctype html><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="${csp}">
    <script nonce="${nonce}" src="${safeUrl}"></script>
    <script nonce="${nonce}">${frameBootstrap}</script>`
}

export function renderMermaid(
  source: string,
  dark: boolean,
  signal: AbortSignal
): Promise<DiagramImage | null> {
  if (source.length > maxSourceLength || signal.aborted) {
    return Promise.resolve(null)
  }
  const header = source.trimStart().replace(/^(?:%%[^\r\n]*[\r\n]+\s*)+/, "")
  if (!diagramHeader.test(header) || mermaidDirective.test(source)) {
    return Promise.resolve(null)
  }

  const frame = document.createElement("iframe")
  frame.setAttribute("sandbox", "allow-scripts")
  frame.setAttribute("aria-hidden", "true")
  frame.tabIndex = -1
  frame.referrerPolicy = "no-referrer"
  frame.style.cssText =
    "position:fixed;inset:0;width:1024px;height:768px;visibility:hidden;pointer-events:none;border:0"
  frame.srcdoc = rendererDocument()

  return new Promise((resolve) => {
    const timeout = window.setTimeout(() => finish(null), renderTimeoutMs)

    function finish(result: DiagramImage | null) {
      window.clearTimeout(timeout)
      window.removeEventListener("message", receive)
      signal.removeEventListener("abort", abort)
      frame.remove()
      resolve(result)
    }

    function abort() {
      finish(null)
    }

    function receive(event: MessageEvent) {
      if (
        event.origin !== "null" ||
        event.source !== frame.contentWindow ||
        event.data?.type !== "factory:rendered-diagram"
      ) {
        return
      }
      finish(imageFromReply(event.data.result))
    }

    frame.addEventListener(
      "load",
      () => {
        frame.contentWindow?.postMessage(
          {
            type: "factory:render-diagram",
            source,
            themeVariables: {
              darkMode: dark,
              background: dark ? "#19191c" : "#f3f3f5",
              primaryColor: dark ? "#29292e" : "#ffffff",
              primaryTextColor: dark ? "#eeeeef" : "#1c1c20",
              primaryBorderColor: dark ? "#a0a0aa" : "#676773",
              lineColor: dark ? "#a0a0aa" : "#676773",
              fontFamily: "monospace",
            },
          },
          // The sandbox deliberately gives the frame an opaque origin.
          "*"
        )
      },
      { once: true }
    )
    window.addEventListener("message", receive)
    signal.addEventListener("abort", abort, { once: true })
    document.body.append(frame)
  })
}
