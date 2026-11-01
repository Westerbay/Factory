import * as m from "@workspace/i18n/messages"
import { useLocale, translateMessage } from "../../lib/i18n"
import { useEffect, useLayoutEffect, useRef, useState } from "react"
import {
  ChevronDown,
  ChevronRight,
  FolderGit2,
  Plus,
  SquarePen,
  Trash2,
} from "lucide-react"

import { Button, IconButton } from "../../components/ui/button"
import { Modal } from "../../components/ui/modal"
import { LoadingState } from "../../components/ui/loading-state"
import type { Conversation } from "../conversations/api"
import { conversationLabel } from "../conversations/conversation-navigation"
import type { ProjectWorkspace } from "./use-projects"
import "./projects.css"

export function ProjectNavigation({
  workspace,
  available,
  busy,
  onNewChat,
  conversations,
  selectedConversation,
  onSelectConversation,
  onProjectRemoved,
}: {
  workspace: ProjectWorkspace
  available: boolean
  busy: boolean
  onNewChat: (projectId: string) => void
  conversations: Conversation[]
  selectedConversation: string | undefined
  onSelectConversation: (id: string) => void
  onProjectRemoved: (projectId: string) => void
}) {
  useLocale()
  const [search, setSearch] = useState("")
  const [deleteTarget, setDeleteTarget] = useState<string>()
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () =>
      new Set(workspace.selectedProjectId ? [workspace.selectedProjectId] : [])
  )
  useEffect(() => {
    const id = workspace.selectedProjectId
    if (!id) return
    setExpandedProjects((current) => {
      if (current.has(id)) return current
      return new Set([...current, id])
    })
  }, [workspace.selectedProjectId])
  const addButton = useRef<HTMLButtonElement>(null)
  const deleteFocus = useRef<HTMLElement>(null)
  const closeButton = useRef<HTMLButtonElement>(null)
  useLayoutEffect(() => {
    // A focused transient control may be removed after an asynchronous reply.
    // Do not move focus when the user is already on another live control.
    if (workspace.pickerOpen && document.activeElement === document.body)
      closeButton.current?.focus()
  }, [
    workspace.pickerOpen,
    workspace.connectionLoading,
    workspace.connection?.status,
    workspace.repositoryLoading,
    workspace.repositoryMessage,
    workspace.busy,
    workspace.importProgress,
  ])
  const matching = workspace.repositories.filter((repository) =>
    repository.fullName
      .toLocaleLowerCase()
      .includes(search.trim().toLocaleLowerCase())
  )
  const progress = workspace.importProgress
  const trackedImport = workspace.projects.find(
    (project) => project.id === progress?.projectId
  )
  const importInterrupted =
    (Boolean(progress) && !available) ||
    progress?.checkFailed ||
    trackedImport?.state === "uncertain" ||
    trackedImport?.state === "failed"
  const target = workspace.projects.find(
    (project) => project.id === deleteTarget
  )
  return (
    <section className="project-navigation" aria-label={m.projects()}>
      <div className="project-navigation-heading">
        <p className="conversation-list-label">{m.projects()}</p>
        <IconButton
          ref={addButton}
          label={m.add_project()}
          disabled={!available || workspace.busy}
          onClick={() => {
            setSearch("")
            workspace.openPicker()
          }}
        >
          <Plus size={16} aria-hidden="true" />
        </IconButton>
      </div>
      {workspace.loading && !workspace.projects.length && (
        <p className="project-hint" role="status">
          {m.loading_projects()}{" "}
        </p>
      )}
      {!workspace.loading && !workspace.projects.length && (
        <p className="project-hint">{m.no_projects_yet()}</p>
      )}
      <nav className="project-list" aria-label={m.your_projects()}>
        {workspace.projects.map((project) => (
          <div className="project-item" key={project.id}>
            <div className="project-row-group">
              <Button
                variant="ghost"
                className="project-row"
                aria-expanded={expandedProjects.has(project.id)}
                onClick={() => {
                  const expanded = expandedProjects.has(project.id)
                  setExpandedProjects((current) => {
                    const next = new Set(current)
                    if (expanded) next.delete(project.id)
                    else next.add(project.id)
                    return next
                  })
                  if (!expanded) workspace.setSelectedProjectId(project.id)
                }}
              >
                {expandedProjects.has(project.id) ? (
                  <ChevronDown aria-hidden="true" size={13} />
                ) : (
                  <ChevronRight aria-hidden="true" size={13} />
                )}
                <FolderGit2 aria-hidden="true" size={15} />
                <span title={project.fullName}>
                  {project.fullName.split("/").at(-1)}
                </span>
              </Button>
              <IconButton
                className="project-create-chat"
                label={m.new_chat_in_name({
                  name: project.fullName,
                })}
                disabled={!available || busy || project.state !== "ready"}
                onClick={() => {
                  workspace.setSelectedProjectId(project.id)
                  setExpandedProjects(
                    (current) => new Set([...current, project.id])
                  )
                  onNewChat(project.id)
                }}
              >
                <SquarePen size={15} aria-hidden="true" />
              </IconButton>
              <IconButton
                className="project-delete"
                label={m.delete_name({ name: project.fullName })}
                disabled={
                  !available ||
                  workspace.busy ||
                  busy ||
                  project.state === "importing" ||
                  project.state === "uncertain"
                }
                onClick={(event) => {
                  deleteFocus.current = event.currentTarget
                  setDeleteTarget(project.id)
                }}
              >
                <Trash2 size={15} aria-hidden="true" />
              </IconButton>
            </div>
            {expandedProjects.has(project.id) && (
              <div
                className="project-chats conversation-list"
                aria-label={m.chats_in_name({
                  name: project.fullName,
                })}
              >
                {conversations
                  .filter(
                    (conversation) => conversation.projectId === project.id
                  )
                  .map((conversation) => (
                    <Button
                      key={conversation.id}
                      variant="ghost"
                      className="conversation-row"
                      aria-current={
                        conversation.id === selectedConversation
                          ? "page"
                          : undefined
                      }
                      title={conversationLabel(conversation)}
                      disabled={busy || !available}
                      onClick={() => onSelectConversation(conversation.id)}
                    >
                      <span>{conversationLabel(conversation)}</span>
                    </Button>
                  ))}
              </div>
            )}
            {project.state !== "ready" && (
              <p className="project-state" role="status">
                {project.state === "importing"
                  ? m.importing()
                  : project.state === "uncertain"
                    ? m.import_interrupted_stop_the_workspace_and_recover_it_before_retry()
                    : project.state === "deleting"
                      ? m.deletion_interrupted_delete_again_to_finish()
                      : m.import_needs_attention()}
              </p>
            )}
            {project.id === workspace.selectedProjectId &&
              project.state === "ready" &&
              workspace.selectedVerification !== "matched" &&
              workspace.selectedVerification !== "checking" && (
                <p className="project-state" role="status">
                  {workspace.selectedVerification === "changed"
                    ? m.the_original_github_address_changed_or_is_inaccessible_check_the()
                    : m.github_address_not_verified_check_your_git_remote_before_pushing()}
                </p>
              )}
            {project.renamedSinceClone && (
              <p className="project-state" role="status">
                {m.repository_renamed_since_import_verify_the_git_remote_before_push()}{" "}
              </p>
            )}
            {project.state === "failed" && (
              <Button
                variant="ghost"
                className="project-retry"
                disabled={!available || workspace.busy}
                onClick={() => workspace.retry(project.id)}
              >
                {m.retry_import()}{" "}
              </Button>
            )}
          </div>
        ))}
      </nav>
      {workspace.nextCursor && (
        <Button
          variant="ghost"
          className="project-more"
          disabled={!available || workspace.busy || workspace.loading}
          onClick={workspace.loadMoreProjects}
        >
          {m.older_projects()}{" "}
        </Button>
      )}
      {workspace.message && (
        <div className="project-message" role="status">
          <p>{translateMessage(workspace.message)}</p>
          {workspace.needsCheck && (
            <Button
              variant="ghost"
              disabled={!available || workspace.busy}
              onClick={workspace.refresh}
            >
              {m.check_projects()}{" "}
            </Button>
          )}
        </div>
      )}
      <Modal
        open={Boolean(target)}
        onClose={() => setDeleteTarget(undefined)}
        title={m.delete_project()}
        returnFocusRef={deleteFocus}
        className="project-delete-dialog"
      >
        <p>
          {m.this_permanently_deletes_name_its_local_folder_in_documents_and_a({
            name: target?.fullName ?? "",
          })}
        </p>
        <div className="project-delete-actions">
          <Button data-autofocus onClick={() => setDeleteTarget(undefined)}>
            {m.cancel()}{" "}
          </Button>
          <Button
            variant="danger"
            disabled={!target || workspace.busy}
            onClick={() => {
              if (!target) return
              const id = target.id
              setDeleteTarget(undefined)
              void workspace.removeProject(id).then((removed) => {
                if (removed) onProjectRemoved(id)
              })
            }}
          >
            {m.delete_project_9c0b617e()}{" "}
          </Button>
        </div>
      </Modal>
      <Modal
        open={workspace.pickerOpen}
        onClose={workspace.closePicker}
        title={progress ? m.import_project() : m.add_a_project()}
        returnFocusRef={addButton}
        className="project-picker"
      >
        {progress ? (
          <div className="project-import-progress">
            {importInterrupted ? (
              <div role="status" aria-live="polite">
                <p className="project-import-title">
                  {m.import_could_not_be_confirmed()}{" "}
                </p>
                <p>
                  {m.check_the_project_list_before_trying_again_if_the_import_was_inte()}{" "}
                </p>
              </div>
            ) : (
              <>
                <LoadingState title={m.importing_project()} />
                <p>
                  {m.cloning_name_into_documents({
                    name: progress.fullName,
                  })}
                </p>
              </>
            )}
          </div>
        ) : workspace.connection?.status === "connected" ? (
          <>
            <p className="project-picker-caption">
              {m.choose_a_github_repository_to_clone_into_your_workspace()}{" "}
            </p>
            <label className="project-search">
              {m.search_repositories()}{" "}
              <input
                data-autofocus
                type="search"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                autoComplete="off"
              />
            </label>
            <div
              className="project-repositories"
              aria-label={m.importable_repositories()}
              aria-busy={workspace.repositoryLoading || workspace.busy}
            >
              {matching.map((repository) => (
                <Button
                  key={repository.id}
                  variant="ghost"
                  className="project-repository"
                  disabled={!available || workspace.busy}
                  onClick={() => workspace.importRepository(repository)}
                >
                  <span>{repository.fullName}</span>
                  <span className="project-repository-meta">
                    <small>
                      {repository.private ? m.private() : m.public()}
                    </small>
                    <ChevronRight size={16} aria-hidden="true" />
                  </span>
                </Button>
              ))}
              {!matching.length &&
                !workspace.repositoryLoading &&
                !workspace.repositoryMessage && (
                  <p className="project-hint">
                    {search
                      ? m.no_matches_in_the_loaded_repositories()
                      : m.no_repositories_to_import()}
                  </p>
                )}
            </div>
            <p className="project-hint">
              {m.search_includes_loaded_repositories()}{" "}
              {workspace.nextPage
                ? m.load_more_to_search_additional_repositories()
                : ""}
            </p>
            {workspace.repositoryLoading && (
              <p className="project-hint" role="status">
                {m.loading_repositories()}{" "}
              </p>
            )}
            {workspace.repositoryMessage && (
              <div className="project-message" role="status">
                <p>{translateMessage(workspace.repositoryMessage)}</p>
                <Button
                  variant="ghost"
                  disabled={workspace.busy}
                  onClick={workspace.refreshRepositories}
                >
                  {m.try_again()}{" "}
                </Button>
                <Button
                  variant="ghost"
                  disabled={workspace.busy}
                  onClick={workspace.connect}
                >
                  {m.reconnect_github()}{" "}
                </Button>
              </div>
            )}
            {workspace.nextPage && (
              <Button
                variant="ghost"
                disabled={workspace.repositoryLoading || workspace.busy}
                onClick={workspace.loadMoreRepositories}
              >
                {m.load_more()}{" "}
              </Button>
            )}
            <details className="project-account">
              <summary>{m.github_access()}</summary>
              <p>
                {m.connected_as_login_git_commits_include_your_github_email_disconne(
                  { login: workspace.connection.login ?? "" }
                )}
              </p>
              <Button
                variant="ghost"
                disabled={workspace.busy}
                onClick={workspace.disconnect}
              >
                {m.disconnect_github_access()}{" "}
              </Button>
              <a
                href="https://github.com/settings/keys"
                target="_blank"
                rel="noreferrer"
              >
                {m.review_ssh_keys_on_github()}{" "}
              </a>
            </details>
          </>
        ) : workspace.connectionLoading && !workspace.connection ? (
          <p className="project-picker-caption" role="status">
            {m.checking_github_access()}{" "}
          </p>
        ) : workspace.connection?.status === "disconnected" ? (
          <div className="project-connect">
            <p>
              {m.connect_the_same_github_account_to_list_and_clone_your_repositori()}{" "}
            </p>
            <p>
              {m.the_github_authorization_grants_factory_broad_read_and_write_acce()}
            </p>
            <Button
              data-autofocus
              variant="primary"
              disabled={!available || workspace.busy}
              onClick={workspace.connect}
            >
              {workspace.busy ? m.connecting() : m.connect_github()}
            </Button>
          </div>
        ) : (
          <div className="project-connect">
            <p role="status">{m.github_connection_status_is_unavailable()}</p>
            <Button
              data-autofocus
              disabled={workspace.busy}
              onClick={workspace.refresh}
            >
              {m.try_again()}{" "}
            </Button>
          </div>
        )}
        <div className="ui-modal-actions">
          <Button ref={closeButton} onClick={workspace.closePicker}>
            {progress && !importInterrupted
              ? m.continue_in_background()
              : m.close()}
          </Button>
        </div>
      </Modal>
    </section>
  )
}
