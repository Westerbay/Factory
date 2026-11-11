import * as m from "@workspace/i18n/messages"
import { useCallback, useEffect, useRef, useState } from "react"

import { useSession } from "../../lib/auth/session-context"
import * as api from "./api"
import { githubCallbackMessage, mergeProjectPage } from "./state"
import { useProjectVerification } from "./use-project-verification"

type ListedRepository = api.Repository & { page: number }
type ImportProgress = {
  fullName: string
  projectId?: string
  checkFailed: boolean
}

/** Private project state lives only for the current verified session. */
export function useProjects(available: boolean) {
  const { state: session } = useSession()
  const userId = session.status === "active" ? session.session.user.id : ""
  const csrf = session.status === "active" ? session.session.csrfToken : ""
  const [projects, setProjects] = useState<api.Project[]>([])
  const [selectedProjectId, setSelectedProjectId] = useState<string>()
  const [connection, setConnection] = useState<api.ProjectConnection>()
  const [connectionLoading, setConnectionLoading] = useState(false)
  const [nextCursor, setNextCursor] = useState<string | null>()
  const [repositories, setRepositories] = useState<ListedRepository[]>([])
  const [nextPage, setNextPage] = useState<number | null>()
  const [pickerOpen, setPickerOpen] = useState(false)
  const [importProgress, setImportProgress] = useState<ImportProgress>()
  const [loading, setLoading] = useState(false)
  const [repositoryLoading, setRepositoryLoading] = useState(false)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState("")
  const [needsCheck, setNeedsCheck] = useState(false)
  const [repositoryMessage, setRepositoryMessage] = useState("")
  const [version, setVersion] = useState(0)
  const [connectionVersion, setConnectionVersion] = useState(0)
  const [repositoryVersion, setRepositoryVersion] = useState(0)
  const lifetime = useRef<AbortController | null>(null)
  const repositoryLifetime = useRef<AbortController | null>(null)
  const loadedProjectPages = useRef(1)
  const projectListGeneration = useRef(0)
  const pendingUpserts = useRef<api.Project[]>([])
  const pending = useRef(false)

  useEffect(() => {
    const controller = new AbortController()
    lifetime.current = controller
    setProjects([])
    setConnection(undefined)
    setSelectedProjectId(undefined)
    loadedProjectPages.current = 1
    pendingUpserts.current = []
    setPickerOpen(false)
    setImportProgress(undefined)
    const url = new URL(window.location.href)
    const callback = url.searchParams.get("github")
    setMessage(callback ? githubCallbackMessage(callback) : "")
    setNeedsCheck(Boolean(callback && callback !== "connected"))
    if (callback) {
      url.searchParams.delete("github")
      window.history.replaceState(window.history.state, "", url)
    }
    return () => {
      controller.abort()
      lifetime.current = null
      pending.current = false
    }
  }, [userId, csrf])

  useEffect(() => {
    if (!available || !csrf) return
    const controller = new AbortController()
    const generation = ++projectListGeneration.current
    setLoading(true)
    const preserved = pendingUpserts.current
    setProjects(preserved)
    setNextCursor(null)
    const pages = loadedProjectPages.current
    const load = async () => {
      let before: string | undefined
      for (let index = 0; index < pages; index++) {
        const page = await api.list(
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
          before
        )
        if (
          controller.signal.aborted ||
          generation !== projectListGeneration.current
        )
          return
        setProjects((current) =>
          mergeProjectPage(current, page.items, index === 0, preserved)
        )
        setNextCursor(page.nextCursor ?? null)
        before = page.nextCursor ?? undefined
        if (!before) break
      }
      if (
        !controller.signal.aborted &&
        generation === projectListGeneration.current
      )
        pendingUpserts.current = []
    }
    void load()
      .catch((error: unknown) => {
        if (
          controller.signal.aborted ||
          generation !== projectListGeneration.current
        )
          return
        setMessage(api.projectProblem(error))
        setNeedsCheck(true)
      })
      .finally(() => {
        if (
          !controller.signal.aborted &&
          generation === projectListGeneration.current
        )
          setLoading(false)
      })
    return () => {
      controller.abort()
      projectListGeneration.current++
    }
  }, [available, csrf, version])

  useEffect(() => {
    if (!available || !csrf) return
    const controller = new AbortController()
    setConnectionLoading(true)
    const signal = AbortSignal.any([
      controller.signal,
      AbortSignal.timeout(30_000),
    ])
    void api
      .ensureConnection(csrf, signal)
      .then((account) => {
        if (!controller.signal.aborted) setConnection(account)
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return
        setConnection(undefined)
        setMessage(api.projectProblem(error))
        setNeedsCheck(true)
      })
      .finally(() => {
        if (!controller.signal.aborted) setConnectionLoading(false)
      })
    return () => controller.abort()
  }, [available, csrf, connectionVersion])

  useEffect(() => {
    if (!available || !pickerOpen || connection?.status !== "connected") return
    const controller = new AbortController()
    repositoryLifetime.current = controller
    setRepositoryLoading(true)
    setRepositoryMessage("")
    setRepositories([])
    setNextPage(undefined)
    void api
      .repositories(1, csrf, controller.signal)
      .then((page) => {
        if (controller.signal.aborted) return
        setRepositories(page.items.map((item) => ({ ...item, page: 1 })))
        setNextPage(page.nextPage ?? null)
        // The provider page can update local names after a rename or transfer.
        setVersion((current) => current + 1)
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted)
          setRepositoryMessage(api.projectProblem(error))
      })
      .finally(() => {
        if (!controller.signal.aborted) setRepositoryLoading(false)
      })
    return () => {
      controller.abort()
      if (repositoryLifetime.current === controller)
        repositoryLifetime.current = null
    }
  }, [available, csrf, pickerOpen, connection?.status, repositoryVersion])

  // Poll only live imports; a lost response becomes an explicit uncertain state.
  useEffect(() => {
    const active = projects.filter((project) => project.state === "importing")
    if (!available || !active.length) return
    const controller = new AbortController()
    let inFlight = false
    const poll = () => {
      if (inFlight || document.visibilityState === "hidden") return
      inFlight = true
      void Promise.allSettled(
        active.map((project) => api.get(project.id, controller.signal))
      )
        .then((results) => {
          if (controller.signal.aborted) return
          const changed = results.flatMap((result) =>
            result.status === "fulfilled" ? [result.value] : []
          )
          setProjects((current) =>
            current.map(
              (project) =>
                changed.find((item) => item.id === project.id) ?? project
            )
          )
          const currentIndex = active.findIndex(
            (project) => project.id === importProgress?.projectId
          )
          if (currentIndex >= 0) {
            const result = results[currentIndex]
            setImportProgress((current) =>
              current && current.projectId === active[currentIndex]?.id
                ? { ...current, checkFailed: result?.status === "rejected" }
                : current
            )
          }
          if (results.some((result) => result.status === "rejected")) {
            setMessage(
              "Import status could not be checked. Check projects again."
            )
            setNeedsCheck(true)
          }
        })
        .finally(() => {
          inFlight = false
        })
    }
    const timer = window.setInterval(poll, 3000)
    return () => {
      controller.abort()
      window.clearInterval(timer)
    }
  }, [available, projects, importProgress?.projectId])

  useEffect(() => {
    if (!importProgress?.projectId) return
    const project = projects.find(
      (item) => item.id === importProgress.projectId
    )
    if (project?.state !== "ready") return
    setImportProgress(undefined)
    setPickerOpen(false)
    setMessage("Project ready.")
    setNeedsCheck(false)
  }, [importProgress?.projectId, projects])

  const run = useCallback(
    async (action: (signal: AbortSignal) => Promise<void>) => {
      if (pending.current || !lifetime.current || !available || !csrf) return
      pending.current = true
      setBusy(true)
      const signal = lifetime.current.signal
      try {
        await action(signal)
      } catch (error) {
        if (!signal.aborted) {
          setMessage(api.projectProblem(error))
          setNeedsCheck(true)
        }
      } finally {
        if (lifetime.current?.signal === signal) {
          if (!signal.aborted) setBusy(false)
          pending.current = false
        }
      }
    },
    [available, csrf]
  )

  function upsert(project: api.Project) {
    // Discard pages observed before this completed mutation, including their cursors.
    projectListGeneration.current++
    pendingUpserts.current = [
      project,
      ...pendingUpserts.current.filter((item) => item.id !== project.id),
    ]
    setVersion((current) => current + 1)
    setProjects((current) => [
      project,
      ...current.filter((item) => item.id !== project.id),
    ])
  }
  function connect() {
    void run(async (signal) => {
      const url = await api.connect(csrf, signal)
      if (!signal.aborted) window.location.assign(url)
    })
  }
  function reconnectInNewTab() {
    if (pending.current || !lifetime.current || !available || !csrf) return
    const tab = window.open("", "_blank")
    if (!tab) {
      setMessage(m.github_reconnect_tab_blocked())
      return
    }
    tab.opener = null
    void run(async (signal) => {
      let redirected = false
      try {
        const url = await api.connect(csrf, signal)
        if (signal.aborted || tab.closed) return
        tab.location.replace(url)
        redirected = true
      } finally {
        if (!redirected) tab.close()
      }
    })
  }
  function disconnect() {
    void run(async (signal) => {
      await api.disconnect(csrf, signal)
      if (signal.aborted) return
      setConnection({ status: "disconnected" })
      setRepositories([])
      setMessage(
        "GitHub repository access disconnected. Existing projects remain available."
      )
      setNeedsCheck(false)
    })
  }
  function importRepository(item: ListedRepository) {
    void run(async (signal) => {
      setImportProgress({ fullName: item.fullName, checkFailed: false })
      setRepositoryMessage("")
      try {
        const project = await api.importRepository(
          item.id,
          item.page,
          csrf,
          signal
        )
        if (signal.aborted) return
        upsert(project)
        setSelectedProjectId(project.id)
        setRepositories((current) =>
          current.filter((repository) => repository.id !== item.id)
        )
        setImportProgress({
          fullName: item.fullName,
          projectId: project.id,
          checkFailed: false,
        })
        setMessage("")
        setNeedsCheck(false)
      } catch (error) {
        if (!signal.aborted) {
          if (api.isDefiniteImportRejection(error)) {
            setImportProgress(undefined)
            setRepositoryMessage(api.projectProblem(error))
          } else {
            setImportProgress({ fullName: item.fullName, checkFailed: true })
            setMessage(
              "Import could not be confirmed. Check the project list before trying again."
            )
            setNeedsCheck(true)
            setVersion((current) => current + 1)
          }
        }
        if (signal.aborted) throw error
      }
    })
  }
  async function removeProject(projectId: string): Promise<boolean> {
    let removed = false
    await run(async (signal) => {
      try {
        await api.remove(projectId, csrf, signal)
        removed = true
      } catch (error) {
        if (signal.aborted) throw error
        // A lost response can follow a completed deletion. Confirm absence before
        // removing local chats; other failures remain retryable and visible.
        if (
          !(
            error instanceof api.ProjectFailure &&
            error.code === "project_in_use"
          )
        ) {
          try {
            await api.get(projectId, signal)
          } catch (lookup) {
            if (lookup instanceof api.ProjectFailure && lookup.status === 404)
              removed = true
          }
        }
        if (!removed) {
          setMessage(
            error instanceof api.ProjectFailure &&
              error.code === "project_in_use"
              ? api.projectProblem(error)
              : "Deletion could not be confirmed. Check the project and retry if needed."
          )
          setNeedsCheck(true)
          setVersion((current) => current + 1)
          return
        }
      }
      if (signal.aborted) return
      projectListGeneration.current++
      pendingUpserts.current = pendingUpserts.current.filter(
        (project) => project.id !== projectId
      )
      setProjects((current) =>
        current.filter((project) => project.id !== projectId)
      )
      setSelectedProjectId((current) =>
        current === projectId ? undefined : current
      )
      setMessage("Project and its chats deleted.")
      setNeedsCheck(false)
      setVersion((current) => current + 1)
    })
    return Boolean(
      removed && lifetime.current && !lifetime.current.signal.aborted
    )
  }
  function retry(projectId: string) {
    void run(async (signal) => {
      const project = await api.retry(projectId, csrf, signal)
      if (!signal.aborted) {
        upsert(project)
        setMessage("Checking the import…")
        setNeedsCheck(false)
      }
    })
  }
  function loadMoreProjects() {
    if (!nextCursor || loading) return
    const cursor = nextCursor
    const generation = projectListGeneration.current
    void run(async (signal) => {
      const page = await api.list(signal, cursor)
      if (signal.aborted || generation !== projectListGeneration.current) return
      setProjects((current) => mergeProjectPage(current, page.items, false))
      setNextCursor(page.nextCursor ?? null)
      loadedProjectPages.current++
    })
  }
  function loadMoreRepositories() {
    if (!nextPage || repositoryLoading || !repositoryLifetime.current) return
    const pageNumber = nextPage
    const signal = repositoryLifetime.current.signal
    setRepositoryLoading(true)
    void api
      .repositories(pageNumber, csrf, signal)
      .then((page) => {
        if (signal.aborted) return
        setRepositories((current) => [
          ...current,
          ...page.items
            .filter(
              (item) => !current.some((existing) => existing.id === item.id)
            )
            .map((item) => ({ ...item, page: pageNumber })),
        ])
        setNextPage(page.nextPage ?? null)
        setRepositoryMessage("")
        setVersion((current) => current + 1)
      })
      .catch((error: unknown) => {
        if (!signal.aborted) setRepositoryMessage(api.projectProblem(error))
      })
      .finally(() => {
        if (!signal.aborted) setRepositoryLoading(false)
      })
  }
  function closePicker() {
    setPickerOpen(false)
    const tracked = projects.find(
      (project) => project.id === importProgress?.projectId
    )
    if (
      importProgress?.checkFailed ||
      (tracked && tracked.state !== "importing")
    ) {
      setImportProgress(undefined)
    }
  }
  function openPicker() {
    setMessage("")
    setNeedsCheck(false)
    setPickerOpen(true)
  }
  function refresh() {
    setMessage("")
    setNeedsCheck(false)
    setVersion((current) => current + 1)
    setConnectionVersion((current) => current + 1)
  }
  function refreshRepositories() {
    setRepositoryVersion((current) => current + 1)
  }
  const selectedVerification = useProjectVerification(
    selectedProjectId,
    available &&
      connection?.status !== "disconnected" &&
      projects.find((project) => project.id === selectedProjectId)?.state ===
        "ready"
  )
  return {
    projects,
    selectedVerification,
    refreshVersion: version,
    selectedProjectId,
    setSelectedProjectId,
    connection,
    connectionLoading,
    pickerOpen,
    setPickerOpen,
    importProgress,
    repositories,
    nextPage,
    nextCursor,
    loading,
    repositoryLoading,
    busy,
    message,
    needsCheck,
    repositoryMessage,
    connect,
    reconnectInNewTab,
    disconnect,
    importRepository,
    removeProject,
    retry,
    loadMoreProjects,
    loadMoreRepositories,
    openPicker,
    closePicker,
    refresh,
    refreshRepositories,
  }
}
export type ProjectWorkspace = ReturnType<typeof useProjects>
