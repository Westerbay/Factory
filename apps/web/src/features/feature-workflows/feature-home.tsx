import * as m from "@workspace/i18n/messages"
import { ArrowRight, GitBranch, MessageSquare, Plus } from "lucide-react"
import { Button } from "../../components/ui/button"
import { LoadingState } from "../../components/ui/loading-state"
import { useLocale } from "../../lib/i18n"
import type { Project } from "../projects/api"
import type { FeatureWorkspaceState } from "./use-feature-workspace"
import { featureStageLabel } from "./feature-workspace"
import "./feature-home.css"

export function FeatureHome({
  state,
  overview,
  available,
  projects,
  projectFilter,
  onProjectFilter,
  onCreate,
  onFeature,
  onChat,
  onFeatures,
}: {
  state: FeatureWorkspaceState
  overview: boolean
  available: boolean
  projects: Project[]
  projectFilter: string
  onProjectFilter: (id: string) => void
  onCreate: () => void
  onFeature: (id: string) => void
  onChat: () => void
  onFeatures: () => void
}) {
  const locale = useLocale()
  const features = overview ? state.features.slice(0, 4) : state.features
  return (
    <section
      className="feature-home"
      aria-label={overview ? m.workspace_overview() : m.features()}
    >
      {state.message && (
        <p className="chat-notice" role="status">
          {state.message}
        </p>
      )}
      {overview ? (
        <div className="feature-home-paths">
          <button type="button" onClick={onChat}>
            <MessageSquare size={21} aria-hidden="true" />
            <h2>{m.chat()}</h2>
            <p>{m.feature_chat_hint()}</p>
            <ArrowRight size={17} aria-hidden="true" />
          </button>
        <button type="button" data-mode="feature" onClick={onFeatures}>
            <GitBranch size={21} aria-hidden="true" />
            <h2>{m.features()}</h2>
            <p>{m.feature_start_hint()}</p>
            <ArrowRight size={17} aria-hidden="true" />
          </button>
        </div>
      ) : (
        <header className="feature-home-heading">
          <div>
            <h2>{m.features()}</h2>
            <p>{m.feature_start_hint()}</p>
          </div>
          <Button
            variant="primary"
            disabled={!available || state.busy}
            onClick={onCreate}
          >
            <Plus size={16} aria-hidden="true" />
            {m.new_feature()}
          </Button>
        </header>
      )}
      <div className="feature-home-list-heading">
        <h3>{overview ? m.feature_recent() : m.feature_all()}</h3>
        {overview ? (
          <Button variant="ghost" onClick={onFeatures}>
            {m.feature_all()}
            <ArrowRight size={14} aria-hidden="true" />
          </Button>
        ) : (
          <label>
            <span className="sr-only">{m.feature_filters()}</span>
            <select
              value={projectFilter}
              onChange={(event) => onProjectFilter(event.target.value)}
            >
              <option value="">{m.feature_all_projects()}</option>
              {projects.map((project) => (
                <option key={project.id} value={project.id}>
                  {project.fullName}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>
      {state.loading && <LoadingState title={m.loading()} />}
      {state.listError && (
        <div className="chat-notice" role="status">
          <p>{m.feature_list_error()}</p>
          <Button disabled={!available} onClick={state.retryList}>
            {m.retry()}
          </Button>
        </div>
      )}
      {!state.loading && !state.listError && features.length === 0 && (
        <div className="feature-home-empty">
          <GitBranch size={25} aria-hidden="true" />
          <p>{m.feature_empty()}</p>
          <Button disabled={!available || state.busy} onClick={onCreate}>
            <Plus size={16} aria-hidden="true" />
            {m.feature_start()}
          </Button>
        </div>
      )}
      <div className="feature-home-list">
        {features.map((feature) => (
          <button
            type="button"
            key={feature.id}
            onClick={() => onFeature(feature.id)}
          >
            <div>
              <strong>{feature.title}</strong>
              <p>
                {projects.find((project) => project.id === feature.projectId)
                  ?.fullName ?? m.feature_project()}{" "}
                <span>·</span>{" "}
                {new Date(feature.createdAt).toLocaleDateString(locale, {
                  month: "short",
                  day: "numeric",
                })}
              </p>
            </div>
            <span className="feature-home-stage">
              {featureStageLabel(feature.stage)}
            </span>
            <ArrowRight size={16} aria-hidden="true" />
          </button>
        ))}
      </div>
      {!overview && state.hasMore && (
        <Button
          className="feature-home-more"
          disabled={state.loadingMore || !available}
          onClick={state.loadMore}
        >
          {state.loadingMore ? m.loading() : m.feature_more()}
        </Button>
      )}
    </section>
  )
}
