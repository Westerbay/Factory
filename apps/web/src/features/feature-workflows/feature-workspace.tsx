import { useEffect, useRef, useState, type ReactNode } from "react"
import { createPortal } from "react-dom"
import * as m from "@workspace/i18n/messages"
import { ArrowLeft, Check, MessageSquare, PanelRight } from "lucide-react"

import { useLocale } from "../../lib/i18n"
import { Button, IconButton } from "../../components/ui/button"
import { SplitPane } from "../../components/ui/split-pane"
import { FeaturePanels } from "./feature-panels"
import type { FeatureStage } from "./api"
import type { FeatureWorkspaceState } from "./use-feature-workspace"
import "./feature-workspace.css"

export function featureStageLabel(stage: FeatureStage) {
  switch (stage) {
    case "scope":
      return m.feature_scope()
    case "architecture":
      return m.feature_architecture()
    case "execution":
      return m.feature_execution()
    case "review":
      return m.feature_review_pending()
    case "result":
      return m.feature_result()
    case "draft":
      return m.feature_draft_receipt()
  }
}

export function FeatureWorkspace({
  state,
  discussion,
  headerHost,
  projectName,
  available,
  onBack,
  onReconnectGitHub,
}: {
  state: FeatureWorkspaceState
  discussion: ReactNode
  headerHost: HTMLElement | null
  projectName?: string
  available: boolean
  onBack: () => void
  onReconnectGitHub: () => void
}) {
  useLocale()
  const feature = state.feature!
  const current = feature.feature.stage
  const [viewStage, setViewStage] = useState<FeatureStage>(current)
  const [compact, setCompact] = useState(false)
  const [showWork, setShowWork] = useState(current !== "scope")
  const [desktopWork, setDesktopWork] = useState(
    Boolean(feature.scope) || current !== "scope"
  )
  const host = useRef<HTMLElement>(null)
  const work = useRef<HTMLDivElement>(null)
  const previousScope = useRef(feature.scope?.id)
  useEffect(() => {
    const element = host.current
    if (!element) return
    const observer = new ResizeObserver(() =>
      setCompact(element.clientWidth < 960)
    )
    setCompact(element.clientWidth < 960)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  useEffect(() => {
    setViewStage(current)
  }, [current])
  useEffect(() => {
    if (!previousScope.current && feature.scope?.id) setDesktopWork(true)
    previousScope.current = feature.scope?.id
  }, [feature.scope?.id])
  useEffect(() => {
    const content = work.current?.querySelector(".feature-panel-content")
    if (content) content.scrollTop = 0
  }, [viewStage])

  const stages: {
    id: FeatureStage
    label: string
    available: boolean
    approved: boolean
  }[] = [
    {
      id: "scope",
      label: m.feature_scope(),
      available: true,
      approved: Boolean(feature.scope?.approvedAt),
    },
    {
      id: "architecture",
      label: m.feature_architecture(),
      available: Boolean(feature.architecture) || current === "architecture",
      approved: Boolean(feature.architecture?.approvedAt),
    },
    {
      id: "execution",
      label: m.feature_execution(),
      available: feature.attempts.length > 0 || current === "execution",
      approved: false,
    },
    {
      id: ["review", "result", "draft"].includes(current) ? current : "result",
      label: m.feature_result(),
      available: Boolean(feature.review),
      approved: false,
    },
  ]
  function selectStage(stage: FeatureStage) {
    setViewStage(stage)
    setShowWork(true)
    setDesktopWork(true)
  }
  function discuss() {
    setShowWork(false)
    requestAnimationFrame(() =>
      host.current?.querySelector("textarea")?.focus()
    )
  }
  return (
    <section
      ref={host}
      className="feature-workspace"
      data-compact={compact ? "" : undefined}
    >
      {headerHost &&
        createPortal(
          <div className="feature-heading">
            <div className="feature-heading-title">
              <IconButton label={m.feature_back()} onClick={onBack}>
                <ArrowLeft size={17} aria-hidden="true" />
              </IconButton>
              <div>
                <h1>{feature.feature.title}</h1>
                <p>{projectName ?? m.feature_project()}</p>
              </div>
            </div>
            <nav className="feature-stages" aria-label={m.feature_stages()}>
              {stages.map((stage, index) => (
                <button
                  type="button"
                  key={stage.id}
                  disabled={!stage.available}
                  aria-current={viewStage === stage.id ? "step" : undefined}
                  onClick={() => selectStage(stage.id)}
                >
                  <span>
                    {stage.approved ? (
                      <Check size={13} aria-hidden="true" />
                    ) : (
                      index + 1
                    )}
                  </span>
                  {stage.label}
                </button>
              ))}
            </nav>
            {!compact && (
              <IconButton
                label={
                  desktopWork ? m.feature_hide_panel() : m.feature_show_panel()
                }
                aria-expanded={desktopWork}
                aria-controls="feature-work-panel"
                onClick={() => setDesktopWork((visible) => !visible)}
              >
                <PanelRight size={17} aria-hidden="true" />
              </IconButton>
            )}
          </div>,
          headerHost
        )}
      {state.message && (
        <div className="workspace-notice" role="status">
          <p>{state.message}</p>
          <Button
            variant="ghost"
            disabled={state.busy || !available}
            onClick={
              state.needsRepositoryReconnect
                ? onReconnectGitHub
                : () => void state.refresh()
            }
          >
            {state.needsRepositoryReconnect
              ? m.reconnect_github()
              : m.feature_refresh()}
          </Button>
        </div>
      )}
      {compact && (
        <div className="feature-mobile-tabs">
          <Button variant="ghost" aria-pressed={!showWork} onClick={discuss}>
            <MessageSquare size={16} aria-hidden="true" />
            {m.feature_discussion()}
          </Button>
          <Button
            variant="ghost"
            aria-pressed={showWork}
            aria-controls="feature-work-panel"
            onClick={() => setShowWork(true)}
          >
            <PanelRight size={16} aria-hidden="true" />
            {featureStageLabel(viewStage)}
          </Button>
        </div>
      )}
      <div className="feature-panes">
        <SplitPane
          compact={compact}
          defaultSecondaryPercent={viewStage === "architecture" ? 56 : 40}
          primary={<div className="feature-discussion">{discussion}</div>}
          secondary={
            <div className="feature-work-panel" ref={work}>
              <FeaturePanels
                feature={feature}
                diff={state.diff}
                diffLoading={state.diffLoading}
                diffError={state.diffError}
                busy={state.busy || !available}
                viewStage={viewStage}
                compact={compact}
                onApprove={(revision) => {
                  void state.approve(revision)
                }}
                onReopen={(revision) => {
                  void state.reopen(revision)
                }}
                onResume={(id) => {
                  void state.resume(id)
                }}
                onReview={(id) => {
                  void state.review(id)
                }}
                onDraft={(id) => {
                  void state.draft(id)
                }}
                onReloadDiff={state.reloadDiff}
                onDiscuss={discuss}
              />
            </div>
          }
          secondaryVisible={compact ? showWork : desktopWork}
          onSecondaryVisibleChange={compact ? setShowWork : setDesktopWork}
          secondaryId="feature-work-panel"
        />
      </div>
    </section>
  )
}
