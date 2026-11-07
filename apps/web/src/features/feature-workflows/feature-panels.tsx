import * as m from "@workspace/i18n/messages"
import { useState } from "react"
import {
  ArrowRight,
  Check,
  Circle,
  ExternalLink,
  FileCode2,
  GitBranch,
  MessageSquare,
  RotateCcw,
  TriangleAlert,
} from "lucide-react"

import { Button } from "../../components/ui/button"
import type { components } from "../../lib/api/schema"
import { useLocale } from "../../lib/i18n"
import { ArtifactPreview } from "../artifacts/artifact-preview"
import { useArtifactPreviewLabels } from "../conversations/artifact-labels"
import { MessageMarkdown } from "../conversations/message-markdown"
import { canPublish, canReconcileDraft, matchesReviewedDiff } from "./state"
import "./feature-panels.css"

type Feature = components["schemas"]["FeatureView"]
type FeatureDiff = components["schemas"]["FeatureDiffView"]
type Attempt = components["schemas"]["FeatureAttemptView"]
type Revision = { id: string; digest: string }
export type FeaturePanelStage = components["schemas"]["FeatureStageView"]

export type FeaturePanelsProps = {
  feature: Feature
  diff?: FeatureDiff
  diffLoading: boolean
  diffError: boolean
  busy: boolean
  viewStage?: FeaturePanelStage
  compact?: boolean
  onApprove: (revision: Revision) => void
  onReopen: (revision: Revision) => void
  onResume: (attemptId?: string) => void
  onReview: (snapshotId: string) => void
  onDraft: (snapshotId: string) => void
  onReloadDiff: () => void
  onDiscuss: () => void
}

export function featureRoleLabel(role: Attempt["role"]): string {
  const labels = {
    coordinator: m.feature_role_coordinator,
    architecture: m.feature_role_architecture,
    implementation: m.feature_role_implementation,
    complexImplementation: m.feature_role_complex_implementation,
    visual: m.feature_role_visual,
    triage: m.feature_role_triage,
    generalReview: m.feature_role_general_review,
    architectureReview: m.feature_role_architecture_review,
    webReview: m.feature_role_web_review,
    securityReview: m.feature_role_security_review,
    correction: m.feature_role_correction,
  }
  return labels[role]()
}

export function featureAttemptLabel(status: Attempt["status"]): string {
  const labels = {
    starting: m.feature_status_starting,
    running: m.feature_status_running,
    validated: m.feature_status_validated,
    failed: m.feature_status_failed,
    interrupted: m.feature_status_interrupted,
    uncertain: m.feature_status_uncertain,
  }
  return labels[status]()
}

function deliveryLabel(status: NonNullable<Feature["delivery"]>["status"]) {
  const labels = {
    claimed: m.feature_delivery_claimed,
    uncertain: m.feature_delivery_uncertain,
    failed: m.feature_delivery_failed,
    published: m.feature_delivery_published,
    publishedStale: m.feature_delivery_stale,
  }
  return labels[status]()
}

function draftLink(value: string | null | undefined): string | undefined {
  if (!value) return undefined
  try {
    const url = new URL(value)
    return url.protocol === "https:" &&
      url.hostname === "github.com" &&
      !url.username &&
      !url.password &&
      !url.port &&
      /^\/[^/]+\/[^/]+\/pull\/\d+\/?$/.test(url.pathname)
      ? url.href
      : undefined
  } catch {
    return undefined
  }
}

function isResultStage(stage: FeaturePanelStage) {
  return stage === "review" || stage === "result" || stage === "draft"
}

function AttemptStatus({ attempt }: { attempt: Attempt }) {
  return (
    <span className="feature-attempt-status" data-status={attempt.status}>
      {attempt.status === "validated" ? (
        <Check size={13} aria-hidden="true" />
      ) : attempt.status === "failed" || attempt.status === "uncertain" ? (
        <TriangleAlert size={13} aria-hidden="true" />
      ) : (
        <Circle size={9} aria-hidden="true" />
      )}
      {featureAttemptLabel(attempt.status)}
    </span>
  )
}

export function FeaturePanels({
  feature,
  diff,
  diffLoading,
  diffError,
  busy,
  viewStage = feature.feature.stage,
  compact = false,
  onApprove,
  onReopen,
  onResume,
  onReview,
  onDraft,
  onReloadDiff,
  onDiscuss,
}: FeaturePanelsProps) {
  useLocale()
  const artifactLabels = useArtifactPreviewLabels()
  const [selectedDiagram, setSelectedDiagram] = useState(0)
  const [selectedAttempt, setSelectedAttempt] = useState<string>()
  const stage = feature.feature.stage
  const current =
    stage === viewStage || (isResultStage(stage) && isResultStage(viewStage))
  const scope = feature.scope
  const architecture = feature.architecture
  const review = feature.review
  const delivery = feature.delivery
  const publishedStale = delivery?.status === "publishedStale"
  const published = delivery?.status === "published" || publishedStale
  const gate = viewStage === "scope" ? scope : architecture
  const diagram =
    architecture?.diagrams[selectedDiagram] ?? architecture?.diagrams[0]
  const attempt =
    feature.attempts.find((item) => item.id === selectedAttempt) ??
    feature.attempts.find((item) => item.canResume) ??
    feature.attempts.find((item) =>
      ["running", "starting", "uncertain"].includes(item.status)
    ) ??
    feature.attempts.at(-1)
  const unsettled = feature.attempts.some((item) =>
    ["starting", "running", "uncertain"].includes(item.status)
  )
  const blocked =
    !unsettled &&
    !published &&
    delivery?.status !== "claimed" &&
    delivery?.status !== "uncertain" &&
    (feature.attempts.some((item) =>
      ["failed", "interrupted"].includes(item.status)
    ) ||
      Boolean(review && !review.ready) ||
      stage === "result")
  const staleDiff = Boolean(
    review && diff && !matchesReviewedDiff(feature, diff)
  )
  const refreshComparison =
    stage === "review" &&
    review?.ready &&
    delivery?.status === "failed" &&
    delivery.snapshotId === review.snapshotId &&
    matchesReviewedDiff(feature, diff)
  const showReviewEvidence =
    !publishedStale &&
    !diffError &&
    !refreshComparison &&
    matchesReviewedDiff(feature, diff)
  const readyForDelivery =
    review?.ready &&
    showReviewEvidence &&
    (feature.feature.stage === "result" || feature.feature.stage === "draft")
  const diffLabel = publishedStale
    ? m.feature_published_diff()
    : staleDiff
      ? m.feature_current_diff()
      : m.feature_diff()
  const identity = diffError ? undefined : diff
  const publishable =
    current && !busy && !diffLoading && !diffError && canPublish(feature, diff)
  const url = draftLink(delivery?.draftUrl)
  const reconcile = canReconcileDraft(feature)

  const reopening = blocked &&
    (current || viewStage === "execution") &&
    (scope?.approvedAt || architecture?.approvedAt) && (
      <div className="feature-panel-reopen">
        {scope?.approvedAt && stage !== "scope" && (
          <Button
            variant="ghost"
            disabled={busy}
            onClick={() => onReopen(scope)}
          >
            <RotateCcw size={14} aria-hidden="true" />
            {m.feature_reopen_scope()}
          </Button>
        )}
        {architecture?.approvedAt &&
          stage !== "scope" &&
          stage !== "architecture" && (
            <Button
              variant="ghost"
              disabled={busy}
              onClick={() => onReopen(architecture)}
            >
              <RotateCcw size={14} aria-hidden="true" />
              {m.feature_reopen_architecture()}
            </Button>
          )}
      </div>
    )

  return (
    <div className="feature-panels" data-stage={viewStage}>
      <div className="feature-panel-content">
        {!current && viewStage !== "execution" && (
          <p className="feature-panel-note">{m.feature_read_only_revision()}</p>
        )}

        {viewStage === "scope" && (
          <>
            {scope ? (
              <>
                <p className="feature-panel-revision">
                  {scope.approvedAt ? (
                    <Check size={14} aria-hidden="true" />
                  ) : (
                    <Circle size={10} aria-hidden="true" />
                  )}
                  {scope.approvedAt
                    ? m.feature_gate_approved()
                    : m.feature_gate_pending()}
                </p>
                <section
                  aria-label={m.feature_product_brief()}
                  className="feature-panel-proposal"
                >
                  <MessageMarkdown>{scope.brief}</MessageMarkdown>
                </section>
              </>
            ) : (
              <p className="feature-panel-empty">{m.feature_scope_empty()}</p>
            )}
          </>
        )}

        {viewStage === "architecture" && (
          <>
            {architecture ? (
              <>
                <p className="feature-panel-revision">
                  {architecture.approvedAt ? (
                    <Check size={14} aria-hidden="true" />
                  ) : (
                    <Circle size={10} aria-hidden="true" />
                  )}
                  {architecture.approvedAt
                    ? m.feature_gate_approved()
                    : m.feature_gate_pending()}
                </p>
                {architecture.summary && (
                  <div className="feature-panel-proposal">
                    <MessageMarkdown>{architecture.summary}</MessageMarkdown>
                  </div>
                )}
                {architecture.diagrams.length > 1 && (
                  <div
                    className="feature-panel-diagram-tabs"
                    aria-label={m.feature_architecture()}
                  >
                    {architecture.diagrams.map((item, index) => (
                      <Button
                        key={`${index}:${item.title}`}
                        variant="ghost"
                        aria-pressed={diagram === item}
                        onClick={() => setSelectedDiagram(index)}
                      >
                        {item.title}
                      </Button>
                    ))}
                  </div>
                )}
                {diagram && (
                  <ArtifactPreview
                    key={`${architecture.id}:${diagram.title}:${selectedDiagram}`}
                    artifact={{ kind: "mermaid", ...diagram }}
                    labels={artifactLabels}
                    compact={compact}
                  />
                )}
              </>
            ) : (
              <p className="feature-panel-empty">
                {m.feature_architecture_empty()}
              </p>
            )}
            {reopening}
          </>
        )}

        {viewStage === "execution" && (
          <>
            {feature.attempts.length === 0 ? (
              <p className="feature-panel-empty">{m.feature_agents_empty()}</p>
            ) : (
              <div className="feature-agent-workspace">
                <ul
                  className="feature-agent-list"
                  aria-label={m.feature_agents()}
                >
                  {feature.attempts.map((item) => (
                    <li key={item.id}>
                      <button
                        type="button"
                        aria-pressed={attempt?.id === item.id}
                        onClick={() => setSelectedAttempt(item.id)}
                      >
                        <strong>{item.title}</strong>
                        <AttemptStatus attempt={item} />
                        <span className="feature-agent-list-meta">
                          {featureRoleLabel(item.role)}
                        </span>
                        <span className="feature-agent-list-model">
                          {item.observedModel ?? item.assignedModel}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
                {attempt && (
                  <article
                    className="feature-agent-detail"
                    aria-label={attempt.title}
                  >
                    <h3>{attempt.title}</h3>
                    <AttemptStatus attempt={attempt} />
                    <dl className="feature-agent-facts">
                      <div>
                        <dt>{m.role()}</dt>
                        <dd>{featureRoleLabel(attempt.role)}</dd>
                      </div>
                      <div>
                        <dt>{m.feature_assigned_model()}</dt>
                        <dd>{attempt.assignedModel}</dd>
                      </div>
                      <div>
                        <dt>{m.feature_observed_model()}</dt>
                        <dd>
                          {attempt.observedModel ??
                            m.feature_model_unconfirmed()}
                        </dd>
                      </div>
                      {attempt.effort && (
                        <div>
                          <dt>{m.reasoning_effort()}</dt>
                          <dd>{attempt.effort}</dd>
                        </div>
                      )}
                    </dl>
                    <section className="feature-agent-section">
                      <h4>{m.feature_activity()}</h4>
                      <p>{attempt.activity || m.feature_activity_empty()}</p>
                    </section>
                    {attempt.checkpoint && (
                      <section className="feature-agent-section">
                        <h4>{m.feature_checkpoint()}</h4>
                        <code>{attempt.checkpoint}</code>
                      </section>
                    )}
                    {attempt.status === "uncertain" && (
                      <p className="feature-panel-alert" role="status">
                        {m.feature_uncertain_attempt()}
                      </p>
                    )}
                    {attempt.canResume && (
                      <Button
                        disabled={busy}
                        onClick={() => onResume(attempt.id)}
                      >
                        <RotateCcw size={14} aria-hidden="true" />
                        {m.feature_resume_attempt()}
                      </Button>
                    )}
                  </article>
                )}
              </div>
            )}
            {reopening}
          </>
        )}

        {isResultStage(viewStage) && (
          <>
            {!review ? (
              <p className="feature-panel-empty">{m.feature_review_empty()}</p>
            ) : (
              <>
                <div className="feature-result-verdict">
                  {readyForDelivery ? (
                    <Check size={18} aria-hidden="true" />
                  ) : (
                    <Circle size={14} aria-hidden="true" />
                  )}
                  <h3>
                    {publishedStale
                      ? m.feature_delivery_stale()
                      : readyForDelivery
                        ? m.feature_review_ready()
                        : m.feature_review_pending()}
                  </h3>
                </div>
                {identity && (
                  <div
                    className="feature-result-identity"
                    aria-label={diffLabel}
                  >
                    <GitBranch size={14} aria-hidden="true" />
                    <code title={identity.baseOid}>
                      {identity.baseOid.slice(0, 8)}
                    </code>
                    <ArrowRight size={13} aria-hidden="true" />
                    <code title={identity.headOid}>
                      {identity.headOid.slice(0, 8)}
                    </code>
                  </div>
                )}
                {showReviewEvidence && (
                  <>
                    <section className="feature-result-section">
                      <h4>
                        {m.feature_findings()}{" "}
                        <span>{review.findings.length}</span>
                      </h4>
                      {review.findings.length === 0 ? (
                        <p className="feature-panel-note">
                          {m.feature_empty_findings()}
                        </p>
                      ) : (
                        <ul className="feature-finding-list">
                          {review.findings.map((finding, index) => (
                            <li key={`${index}:${finding.role}`}>
                              <div>
                                <span>{featureRoleLabel(finding.role)}</span>
                                {finding.blocking && (
                                  <strong>
                                    <TriangleAlert
                                      size={13}
                                      aria-hidden="true"
                                    />
                                    {m.feature_blocking()}
                                  </strong>
                                )}
                              </div>
                              <p>{finding.summary}</p>
                            </li>
                          ))}
                        </ul>
                      )}
                    </section>
                    <section className="feature-result-section">
                      <h4>{m.feature_checks()}</h4>
                      {review.checks.length === 0 ? (
                        <p className="feature-panel-note">
                          {m.feature_empty_checks()}
                        </p>
                      ) : (
                        <ul className="feature-check-list">
                          {review.checks.map((check, index) => (
                            <li key={`${index}:${check.label}`}>
                              <span>{check.label}</span>
                              <span data-passed={check.passed}>
                                {check.passed ? (
                                  <Check size={14} aria-hidden="true" />
                                ) : (
                                  <TriangleAlert size={14} aria-hidden="true" />
                                )}
                                {check.passed
                                  ? m.feature_check_passed()
                                  : m.feature_check_failed()}
                              </span>
                            </li>
                          ))}
                        </ul>
                      )}
                    </section>
                  </>
                )}
                {review.canExtend && !published && !reconcile && (
                  <Button
                    disabled={busy || unsettled || staleDiff}
                    onClick={() => onReview(review.snapshotId)}
                  >
                    <RotateCcw size={14} aria-hidden="true" />
                    {m.feature_extend_review()}
                  </Button>
                )}
                <section className="feature-result-section">
                  <h4>
                    {m.feature_changes()}{" "}
                    {diff && <span>{diff.changedPaths.length}</span>}
                  </h4>
                  {diffLoading && (
                    <p className="feature-panel-note" role="status">
                      {m.loading()}
                    </p>
                  )}
                  {diffError && (
                    <div className="feature-panel-alert" role="status">
                      <p>
                        {publishedStale
                          ? m.feature_published_diff_unavailable()
                          : m.feature_diff_unavailable()}
                      </p>
                      <Button
                        variant="ghost"
                        disabled={busy || diffLoading}
                        onClick={onReloadDiff}
                      >
                        {m.retry()}
                      </Button>
                    </div>
                  )}
                  {staleDiff && !published && (
                    <div className="feature-panel-alert" role="status">
                      <p>
                        {reconcile
                          ? m.feature_reconcile_stale_diff()
                          : m.feature_diff_stale()}
                      </p>
                      {!reconcile &&
                        review.ready &&
                        (stage === "result" || stage === "review") && (
                          <Button
                            disabled={busy || unsettled}
                            onClick={() => onReview(review.snapshotId)}
                          >
                            <RotateCcw size={14} aria-hidden="true" />
                            {m.feature_review_current_diff()}
                          </Button>
                        )}
                      <Button
                        variant="ghost"
                        disabled={busy || diffLoading}
                        onClick={onReloadDiff}
                      >
                        {m.feature_refresh()}
                      </Button>
                    </div>
                  )}
                  {diff && !diffError && (
                    <>
                      {diff.changedPaths.length === 0 ? (
                        <p className="feature-panel-note">
                          {m.feature_no_changes()}
                        </p>
                      ) : (
                        <ul className="feature-changed-files">
                          {diff.changedPaths.map((path) => (
                            <li key={path}>
                              <FileCode2 size={14} aria-hidden="true" />
                              <code>{path}</code>
                            </li>
                          ))}
                        </ul>
                      )}
                      <details className="feature-diff" open>
                        <summary>{diffLabel}</summary>
                        <pre tabIndex={0} aria-label={diffLabel}>
                          <code>{diff.patch}</code>
                        </pre>
                      </details>
                    </>
                  )}
                  {refreshComparison && (
                    <Button
                      disabled={busy || unsettled || diffLoading}
                      onClick={() => onReview(review.snapshotId)}
                    >
                      <RotateCcw size={14} aria-hidden="true" />
                      {m.feature_review_refresh_comparison()}
                    </Button>
                  )}
                </section>
              </>
            )}
            {delivery && (
              <section className="feature-result-section feature-delivery">
                <h4>{m.feature_draft_receipt()}</h4>
                <p className="feature-delivery-status">
                  {deliveryLabel(delivery.status)}
                </p>
                {delivery.reason && <p>{delivery.reason}</p>}
                {url && (
                  <a href={url} target="_blank" rel="noopener noreferrer">
                    {m.feature_open_draft()}
                    <ExternalLink size={14} aria-hidden="true" />
                  </a>
                )}
              </section>
            )}
            {reopening}
          </>
        )}
        {current && feature.canContinue && !refreshComparison && (
          <Button disabled={busy} onClick={() => onResume()}>
            <RotateCcw size={14} aria-hidden="true" />
            {m.feature_resume_step()}
          </Button>
        )}
      </div>

      {(viewStage === "scope" || viewStage === "architecture") &&
        (stage === "scope" || stage === "architecture") && (
          <footer className="feature-panel-footer">
            <Button variant="ghost" onClick={onDiscuss}>
              <MessageSquare size={14} aria-hidden="true" />
              {m.feature_discuss()}
            </Button>
            {current && gate && !gate.approvedAt && (
              <>
                <p>
                  {gate.canApprove
                    ? m.feature_gate_revision_note()
                    : m.feature_gate_blocked()}
                </p>
                <Button
                  variant="primary"
                  disabled={busy || !gate.canApprove}
                  onClick={() => onApprove(gate)}
                >
                  {viewStage === "scope"
                    ? m.feature_approve_scope()
                    : m.feature_approve_architecture()}
                  <ArrowRight size={14} aria-hidden="true" />
                </Button>
              </>
            )}
          </footer>
        )}
      {isResultStage(viewStage) &&
        (reconcile || (review && stage === "result")) &&
        current &&
        !published && (
          <footer className="feature-panel-footer">
            {!reconcile && <p>{m.feature_draft_scope()}</p>}
            <Button
              variant="primary"
              disabled={reconcile ? busy : !publishable}
              onClick={() =>
                onDraft(reconcile ? delivery!.snapshotId : review!.snapshotId)
              }
            >
              {reconcile
                ? m.feature_reconcile_draft()
                : m.feature_publish_draft()}
              <ArrowRight size={14} aria-hidden="true" />
            </Button>
          </footer>
        )}
    </div>
  )
}
