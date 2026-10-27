import type {
  DesktopCommand,
  DesktopResult,
  RunProjection,
  MaterialEvent,
} from "@workspace/contracts"
import type {
  BridgeEvent,
  HostStatus,
  WorkspaceSelection,
} from "@workspace/contracts/desktop"
type ProbeResult = Extract<DesktopResult, { type: "probe" }>
type SkillsResult = Extract<DesktopResult, { type: "skills" }>
export interface OperatorState {
  host: HostStatus
  workspace?: WorkspaceSelection
  probe?: ProbeResult["probe"]
  skills: SkillsResult["skills"]
  runs: RunProjection[]
  nextOffset?: number
  pageOffset: number
  activeRunId?: string
  selectedId?: string
  run?: RunProjection
  history: MaterialEvent[]
  nextSequence?: number
  deltas: Record<string, { agentId: string; text: string }>
  busy: string[]
  error?: string
}
const TRANSIENT_TEXT_LIMIT = 262_144
const TRANSIENT_MESSAGE_LIMIT = 32
export class OperatorModel {
  private state: OperatorState = {
    host: { state: "starting", message: "Connecting to local runtime…" },
    skills: [],
    runs: [],
    pageOffset: 0,
    history: [],
    deltas: {},
    busy: [],
  }
  private listeners = new Set<() => void>()
  private selectionVersion = 0
  private workspaceVersion = 0
  private statusVersion = 0
  private initialized = false
  private activeProjection?: RunProjection
  private projectionRevision = 0
  private disconnect?: () => void
  getSnapshot = () => this.state
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }
  private update(change: Partial<OperatorState>) {
    this.state = { ...this.state, ...change }
    for (const listener of this.listeners) listener()
  }
  connect() {
    this.disconnect?.()
    this.disconnect = window.factory.subscribe(this.receive)
    const version = this.statusVersion
    void window.factory
      .status()
      .then((host) => {
        if (version === this.statusVersion) this.host(host)
      })
      .catch(() =>
        this.update({
          error: "The secure desktop bridge is unavailable. Restart Factory.",
        })
      )
  }
  private host(host: HostStatus) {
    this.update({
      host,
      ...(host.state === "unavailable" ? { deltas: {} } : {}),
    })
    if (host.state === "ready" && !this.initialized) {
      this.initialized = true
      void this.refresh()
      void this.probe()
    }
  }
  private receive = (event: BridgeEvent) => {
    if (event.type === "host-status") {
      this.statusVersion++
      this.host(event.status)
      return
    }
    if (event.type === "diagnostic") {
      this.update({
        error: event.message,
        ...(event.code === "persistence"
          ? {
              host: { state: "unavailable", message: event.message },
              deltas: {},
            }
          : {}),
      })
      return
    }
    if (event.type === "run-state") {
      this.mergeRun(event.run)
      return
    }
    if (
      event.type === "message-delta" &&
      this.state.selectedId === event.runId
    ) {
      if (
        this.state.run?.messages.some(
          (message) => message.id === event.messageId
        )
      )
        return
      const prior = this.state.deltas[event.messageId]
      const deltas = {
        ...this.state.deltas,
        [event.messageId]: {
          agentId: event.agentId,
          text: ((prior?.text ?? "") + event.delta).slice(
            -TRANSIENT_TEXT_LIMIT
          ),
        },
      }
      const entries = Object.entries(deltas)
      while (
        entries.length > TRANSIENT_MESSAGE_LIMIT ||
        entries.reduce((sum, entry) => sum + entry[1].text.length, 0) >
          TRANSIENT_TEXT_LIMIT
      ) {
        const oldest = entries.shift()
        if (!oldest) break
        delete deltas[oldest[0]]
      }
      this.update({ deltas })
    }
  }
  private knownRun(id: string): RunProjection | undefined {
    const candidates = [
      this.state.runs.find((run) => run.id === id),
      this.state.run,
      this.activeProjection,
    ]
    let latest: RunProjection | undefined
    for (const candidate of candidates) {
      if (
        candidate?.id === id &&
        (!latest || candidate.version > latest.version)
      )
        latest = candidate
    }
    return latest
  }
  private observeActive(run: RunProjection) {
    const terminal = ["completed", "failed", "interrupted"].includes(run.status)
    const sameRoot = this.activeProjection?.id === run.id
    const priorTerminal =
      this.activeProjection &&
      ["completed", "failed", "interrupted"].includes(
        this.activeProjection.status
      )
    if (sameRoot || (!terminal && (!this.activeProjection || priorTerminal)))
      this.activeProjection = run
  }
  private activeRunId() {
    const run = this.activeProjection
    return run && !["completed", "failed", "interrupted"].includes(run.status)
      ? run.id
      : undefined
  }
  private mergeRun(run: RunProjection, readRevision?: number) {
    const known = this.knownRun(run.id)
    if (known && known.version > run.version) return
    const nonterminal = !["completed", "failed", "interrupted"].includes(
      run.status
    )
    const supersededRead =
      readRevision !== undefined && readRevision !== this.projectionRevision
    if (nonterminal && supersededRead && this.activeProjection?.id !== run.id)
      return
    this.projectionRevision++
    this.observeActive(run)
    const activeRunId = this.activeRunId()
    const existing = this.state.runs.find((item) => item.id === run.id)
    let runs = this.state.runs.map((item) => (item.id === run.id ? run : item))
    if (!existing && this.state.pageOffset === 0)
      runs = [run, ...runs].slice(0, 50)
    if (this.state.selectedId !== run.id) {
      this.update({ runs, activeRunId })
      return
    }
    const deltas = { ...this.state.deltas }
    for (const message of run.messages) delete deltas[message.id]
    if (["completed", "failed", "interrupted"].includes(run.status))
      for (const key of Object.keys(deltas)) delete deltas[key]
    this.update({ runs, run, deltas, activeRunId })
  }
  async command(
    command: DesktopCommand,
    key: string
  ): Promise<DesktopResult | undefined> {
    if (this.state.busy.includes(key)) return
    this.update({ busy: [...this.state.busy, key], error: undefined })
    try {
      const result = await window.factory.command(command)
      if (result.type === "error") this.update({ error: result.message })
      return result
    } catch {
      this.update({
        error:
          "Desktop communication failed. The operation outcome is unconfirmed; refresh before retrying.",
      })
      return
    } finally {
      this.update({ busy: this.state.busy.filter((item) => item !== key) })
    }
  }
  async selectWorkspace() {
    const version = ++this.workspaceVersion
    try {
      const result = await window.factory.selectWorkspace()
      if (version !== this.workspaceVersion) return
      if (result.type === "workspace") {
        this.update({ workspace: result.workspace, skills: [] })
        await this.skills()
      } else if (result.type === "error") this.update({ error: result.message })
    } catch {
      this.update({ error: "Workspace selection failed. Try again." })
    }
  }
  async skills() {
    const workspace = this.state.workspace
    if (!workspace) return
    const result = await this.command(
      { type: "list-skills", workspaceId: workspace.id },
      `skills-${workspace.id}`
    )
    if (result?.type === "skills" && this.state.workspace?.id === workspace.id)
      this.update({ skills: result.skills })
  }
  async probe() {
    const result = await this.command({ type: "probe" }, "probe")
    if (result?.type === "probe") this.update({ probe: result.probe })
  }
  async refresh(offset = 0) {
    const projectionRevision = this.projectionRevision
    const result = await this.command(
      { type: "list-runs", offset, limit: 50 },
      "runs"
    )
    if (result?.type === "runs") {
      const runs = result.runs.map((run) => {
        const current = this.knownRun(run.id)
        return current && current.version > run.version ? current : run
      })
      // A page read issued before a newer observation cannot become active-root authority.
      if (offset === 0 && projectionRevision === this.projectionRevision) {
        for (const run of runs) this.observeActive(run)
      }
      const activeRunId = this.activeRunId()
      this.projectionRevision++
      this.update({
        runs,
        nextOffset: result.nextOffset,
        pageOffset: offset,
        activeRunId,
      })
    }
  }
  async selectRun(id: string) {
    const version = ++this.selectionVersion
    this.update({
      selectedId: id,
      run: this.knownRun(id),
      history: [],
      nextSequence: undefined,
      deltas: {},
    })
    const readRevision = this.projectionRevision
    const result = await this.command(
      { type: "get-run", runId: id },
      `run-${id}`
    )
    if (version !== this.selectionVersion) return
    if (result?.type === "run") this.mergeRun(result.run, readRevision)
    await this.history(false)
  }
  async history(more = false) {
    const id = this.state.selectedId
    if (!id) return
    const version = this.selectionVersion
    const result = await this.command(
      {
        type: "history",
        runId: id,
        afterSequence: more ? (this.state.nextSequence ?? 0) : 0,
        limit: 100,
      },
      `history-${id}`
    )
    if (version !== this.selectionVersion || result?.type !== "history") return
    this.update({
      history: more
        ? [...this.state.history, ...result.events].slice(-1000)
        : result.events,
      nextSequence: result.nextSequence,
    })
  }
  async start(task: string, skillId: string) {
    const workspace = this.state.workspace,
      probe = this.state.probe
    if (!workspace || !probe?.available) return
    const readRevision = this.projectionRevision
    const result = await this.command(
      {
        type: "start-run",
        workspaceId: workspace.id,
        providerId: probe.providerId,
        task,
        ...(skillId ? { skillId } : {}),
      },
      "start"
    )
    if (result?.type === "run") {
      this.mergeRun(result.run, readRevision)
      await this.selectRun(result.run.id)
    }
  }
  async continue(agentId: string, text: string) {
    const run = this.state.run
    if (!run) return
    const readRevision = this.projectionRevision
    const result = await this.command(
      { type: "continue-run", runId: run.id, agentId, text },
      "continue"
    )
    if (result?.type === "run") {
      this.mergeRun(result.run, readRevision)
      await this.selectRun(result.run.id)
    }
  }
  async stop() {
    const run = this.state.run
    if (!run) return
    const readRevision = this.projectionRevision
    const result = await this.command(
      { type: "cancel-run", runId: run.id },
      "stop"
    )
    if (result?.type === "run") this.mergeRun(result.run, readRevision)
  }
}
export const operator = new OperatorModel()
