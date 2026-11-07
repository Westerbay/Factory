import * as m from "@workspace/i18n/messages"
import { useLocale } from "../../lib/i18n"
import type { ArtifactPreviewLabels } from "../artifacts/artifact-preview"

/** Shared labels for typed feature artifacts and opted-in chat previews. */
export function useArtifactPreviewLabels(): ArtifactPreviewLabels {
  useLocale()
  return {
    source: m.artifact_source(),
    hideSource: m.artifact_hide_source(),
    preview: m.artifact_preview(),
    diagramLoading: m.artifact_diagram_loading(),
    diagramError: m.artifact_diagram_error(),
    htmlError: m.artifact_html_error(),
    fit: m.artifact_fit(),
    fitted: m.artifact_fitted(),
    zoomOut: m.artifact_zoom_out(),
    zoomIn: m.artifact_zoom_in(),
    expand: m.artifact_expand(),
    close: m.close(),
    desktop: m.artifact_desktop(),
    mobile: m.artifact_mobile(),
    scrollable: m.artifact_scrollable(),
  }
}
