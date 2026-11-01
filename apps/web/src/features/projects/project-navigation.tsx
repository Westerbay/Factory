import { useTranslation } from "react-i18next"
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
  const { t: translate } = useTranslation()
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
    <section className="project-navigation" aria-label={translate("Projects")}>
      <div className="project-navigation-heading">
        <p className="conversation-list-label">{translate("Projects")}</p>
        <IconButton
          ref={addButton}
          label={translate("Add project")}
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
          {translate("Loading projects…")}{" "}
        </p>
      )}
      {!workspace.loading && !workspace.projects.length && (
        <p className="project-hint">{translate("No projects yet.")}</p>
      )}
      <nav className="project-list" aria-label={translate("Your projects")}>
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
                label={translate("New chat in {{name}}", {
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
                label={translate("Delete {{name}}", { name: project.fullName })}
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
                aria-label={translate("Chats in {{name}}", {
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
                  ? translate("Importing…")
                  : project.state === "uncertain"
                    ? translate(
                        "Import interrupted. Stop the workspace and recover it before retrying."
                      )
                    : project.state === "deleting"
                      ? translate(
                          "Deletion interrupted. Delete again to finish."
                        )
                      : translate("Import needs attention.")}
              </p>
            )}
            {project.id === workspace.selectedProjectId &&
              project.state === "ready" &&
              workspace.selectedVerification !== "matched" &&
              workspace.selectedVerification !== "checking" && (
                <p className="project-state" role="status">
                  {workspace.selectedVerification === "changed"
                    ? translate(
                        "The original GitHub address changed or is inaccessible. Check the Git remote before pushing."
                      )
                    : translate(
                        "GitHub address not verified; check your Git remote before pushing."
                      )}
                </p>
              )}
            {project.renamedSinceClone && (
              <p className="project-state" role="status">
                {translate(
                  "Repository renamed since import. Verify the Git remote before pushing."
                )}{" "}
              </p>
            )}
            {project.state === "failed" && (
              <Button
                variant="ghost"
                className="project-retry"
                disabled={!available || workspace.busy}
                onClick={() => workspace.retry(project.id)}
              >
                {translate("Retry import")}{" "}
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
          {translate("Older projects")}{" "}
        </Button>
      )}
      {workspace.message && (
        <div className="project-message" role="status">
          <p>{translate(workspace.message)}</p>
          {workspace.needsCheck && (
            <Button
              variant="ghost"
              disabled={!available || workspace.busy}
              onClick={workspace.refresh}
            >
              {translate("Check projects")}{" "}
            </Button>
          )}
        </div>
      )}
      <Modal
        open={Boolean(target)}
        onClose={() => setDeleteTarget(undefined)}
        title={translate("Delete project?")}
        returnFocusRef={deleteFocus}
        className="project-delete-dialog"
      >
        <p>
          {translate(
            "This permanently deletes {{name}}, its local folder in Documents, and all its chats. Unpushed changes are lost. Stop terminal commands before deleting. The GitHub repository stays intact.",
            { name: target?.fullName }
          )}
        </p>
        <div className="project-delete-actions">
          <Button data-autofocus onClick={() => setDeleteTarget(undefined)}>
            {translate("Cancel")}{" "}
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
            {translate("Delete project")}{" "}
          </Button>
        </div>
      </Modal>
      <Modal
        open={workspace.pickerOpen}
        onClose={workspace.closePicker}
        title={
          progress ? translate("Import project") : translate("Add a project")
        }
        returnFocusRef={addButton}
        className="project-picker"
      >
        {progress ? (
          <div className="project-import-progress">
            {importInterrupted ? (
              <div role="status" aria-live="polite">
                <p className="project-import-title">
                  {translate("Import could not be confirmed")}{" "}
                </p>
                <p>
                  {translate(
                    "Check the project list before trying again. If the import was interrupted, stop the workspace and recover it before retrying."
                  )}{" "}
                </p>
              </div>
            ) : (
              <>
                <LoadingState title={translate("Importing project…")} />
                <p>
                  {translate("Cloning {{name}} into Documents.", {
                    name: progress.fullName,
                  })}
                </p>
              </>
            )}
          </div>
        ) : workspace.connection?.status === "connected" ? (
          <>
            <p className="project-picker-caption">
              {translate(
                "Choose a GitHub repository to clone into your workspace."
              )}{" "}
            </p>
            <label className="project-search">
              {translate("Search repositories")}{" "}
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
              aria-label={translate("Importable repositories")}
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
                      {repository.private
                        ? translate("Private")
                        : translate("Public")}
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
                      ? translate("No matches in the loaded repositories.")
                      : translate("No repositories to import.")}
                  </p>
                )}
            </div>
            <p className="project-hint">
              {translate("Search includes loaded repositories.")}{" "}
              {workspace.nextPage
                ? translate("Load more to search additional repositories.")
                : ""}
            </p>
            {workspace.repositoryLoading && (
              <p className="project-hint" role="status">
                {translate("Loading repositories…")}{" "}
              </p>
            )}
            {workspace.repositoryMessage && (
              <div className="project-message" role="status">
                <p>{translate(workspace.repositoryMessage)}</p>
                <Button
                  variant="ghost"
                  disabled={workspace.busy}
                  onClick={workspace.refreshRepositories}
                >
                  {translate("Try again")}{" "}
                </Button>
                <Button
                  variant="ghost"
                  disabled={workspace.busy}
                  onClick={workspace.connect}
                >
                  {translate("Reconnect GitHub")}{" "}
                </Button>
              </div>
            )}
            {workspace.nextPage && (
              <Button
                variant="ghost"
                disabled={workspace.repositoryLoading || workspace.busy}
                onClick={workspace.loadMoreRepositories}
              >
                {translate("Load more")}{" "}
              </Button>
            )}
            <details className="project-account">
              <summary>{translate("GitHub access")}</summary>
              <p>
                {translate(
                  "Connected as {{login}}. Git commits include your GitHub email. Disconnecting here keeps your projects and may leave the SSH key active on GitHub.",
                  { login: workspace.connection.login }
                )}
              </p>
              <Button
                variant="ghost"
                disabled={workspace.busy}
                onClick={workspace.disconnect}
              >
                {translate("Disconnect GitHub access")}{" "}
              </Button>
              <a
                href="https://github.com/settings/keys"
                target="_blank"
                rel="noreferrer"
              >
                {translate("Review SSH keys on GitHub")}{" "}
              </a>
            </details>
          </>
        ) : workspace.connectionLoading && !workspace.connection ? (
          <p className="project-picker-caption" role="status">
            {translate("Checking GitHub access…")}{" "}
          </p>
        ) : workspace.connection?.status === "disconnected" ? (
          <div className="project-connect">
            <p>
              {translate(
                "Connect the same GitHub account to list and clone your repositories."
              )}{" "}
            </p>
            <p>
              {translate(
                "The GitHub authorization grants Factory broad read and write access to your private repositories and some organization resources. It also allows Factory to add an SSH key and read your GitHub email. The SSH key lets the workspace use your GitHub permissions. Your commit email is visible to anyone who can read those commits."
              )}
            </p>
            <Button
              data-autofocus
              variant="primary"
              disabled={!available || workspace.busy}
              onClick={workspace.connect}
            >
              {workspace.busy
                ? translate("Connecting…")
                : translate("Connect GitHub")}
            </Button>
          </div>
        ) : (
          <div className="project-connect">
            <p role="status">
              {translate("GitHub connection status is unavailable.")}
            </p>
            <Button
              data-autofocus
              disabled={workspace.busy}
              onClick={workspace.refresh}
            >
              {translate("Try again")}{" "}
            </Button>
          </div>
        )}
        <div className="ui-modal-actions">
          <Button ref={closeButton} onClick={workspace.closePicker}>
            {progress && !importInterrupted
              ? translate("Continue in background")
              : translate("Close")}
          </Button>
        </div>
      </Modal>
    </section>
  )
}
