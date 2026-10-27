import {
  useState,
  useSyncExternalStore,
  type ChangeEvent,
  type FormEvent,
} from "react"
import { operator } from "./model"
import { ExecutionView } from "./ExecutionView"
import { messages as m } from "./messages"
export function App() {
  const state = useSyncExternalStore(operator.subscribe, operator.getSnapshot)
  const [task, setTask] = useState("")
  const [skillId, setSkillId] = useState("")
  const unavailable = state.host.state !== "ready"
  const active = Boolean(state.activeRunId)
  const canStart =
    !unavailable &&
    state.workspace &&
    state.probe?.available &&
    !active &&
    !state.busy.includes("start") &&
    task.trim()
  const selectedSkill = state.skills.some((skill) => skill.id === skillId)
    ? skillId
    : ""
  function selectWorkspace() {
    void operator.selectWorkspace()
  }
  function probe() {
    void operator.probe()
  }
  function skills() {
    void operator.skills()
  }
  function refresh() {
    void operator.refresh()
  }
  function previous() {
    void operator.refresh(Math.max(0, state.pageOffset - 50))
  }
  function more() {
    if (state.nextOffset !== undefined) void operator.refresh(state.nextOffset)
  }
  function changeTask(event: ChangeEvent<HTMLTextAreaElement>) {
    setTask(event.target.value)
  }
  function changeSkill(event: ChangeEvent<HTMLSelectElement>) {
    setSkillId(event.target.value)
  }
  function start(event: FormEvent) {
    event.preventDefault()
    void operator.start(task, selectedSkill)
  }
  function selectRun(event: React.MouseEvent<HTMLButtonElement>) {
    const id = event.currentTarget.dataset.runId
    if (id) void operator.selectRun(id)
  }
  const skillOptions = state.skills.map((skill) => (
    <option value={skill.id} key={skill.id}>
      {skill.reference.name}
    </option>
  ))
  const runs = state.runs.map((run) => (
    <li key={run.id}>
      <button
        data-run-id={run.id}
        onClick={selectRun}
        aria-current={state.selectedId === run.id ? "true" : undefined}
      >
        <strong>{run.task.slice(0, 100)}</strong>
        <span>
          {run.status} · {run.createdAt}
        </span>
      </button>
    </li>
  ))
  return (
    <main>
      <header>
        <p>{m.subtitle}</p>
        <h1>{m.title}</h1>
      </header>
      <section aria-labelledby="runtime-title">
        <h2 id="runtime-title">{m.runtime}</h2>
        <p role="status">{state.host.message}</p>
        <div className="actions">
          <strong>
            {state.probe
              ? state.probe.available
                ? m.providerReady
                : m.providerUnavailable
              : m.providerUnknown}
          </strong>
          <button
            onClick={probe}
            disabled={unavailable || state.busy.includes("probe")}
          >
            {m.checkProvider}
          </button>
        </div>
        {state.probe?.reason && <p>{state.probe.reason}</p>}
        {state.probe && (
          <details>
            <summary>{m.capabilities}</summary>
            <dl>
              <dt>{m.capabilityDelegation}</dt>
              <dd>{String(state.probe.capabilities.nativeDelegation)}</dd>
              <dt>{m.capabilityQuestions}</dt>
              <dd>{String(state.probe.capabilities.structuredQuestions)}</dd>
              <dt>{m.capabilityResume}</dt>
              <dd>{String(state.probe.capabilities.resume)}</dd>
            </dl>
          </details>
        )}
      </section>
      {state.error && (
        <p role="alert" className="error">
          {state.error}
        </p>
      )}
      <div className="workspace-grid">
        <aside>
          <section>
            <h2>{m.workspace}</h2>
            <p>{state.workspace?.name ?? m.noneSelected}</p>
            <button onClick={selectWorkspace} disabled={unavailable}>
              {m.chooseWorkspace}
            </button>
            <p className="meta">{m.workspaceScope}</p>
            <form onSubmit={start}>
              <label>
                {m.skill}
                <select
                  value={selectedSkill}
                  onChange={changeSkill}
                  disabled={unavailable || !state.workspace}
                >
                  <option value="">{m.noSkill}</option>
                  {skillOptions}
                </select>
              </label>
              <ul className="skill-list">
                {state.skills.map((skill) => (
                  <li key={skill.id}>
                    <strong>{skill.reference.name}</strong>
                    <code>{skill.reference.path}</code>
                  </li>
                ))}
              </ul>
              <p className="meta">{m.skillLimit}</p>
              <button
                type="button"
                onClick={skills}
                disabled={
                  unavailable ||
                  !state.workspace ||
                  state.busy.some((key) => key.startsWith("skills-"))
                }
              >
                {m.refreshSkills}
              </button>
              <label>
                {m.task}
                <textarea
                  value={task}
                  onChange={changeTask}
                  maxLength={65536}
                  required
                />
              </label>
              {active && <p>{m.activeWarning}</p>}
              <button type="submit" disabled={!canStart}>
                {m.start}
              </button>
            </form>
          </section>
          <section>
            <h2>{m.history}</h2>
            <button
              onClick={refresh}
              disabled={unavailable || state.busy.includes("runs")}
            >
              {m.refresh}
            </button>
            {runs.length ? (
              <ul className="run-list">{runs}</ul>
            ) : (
              <p>{m.noHistory}</p>
            )}
            {state.pageOffset > 0 && (
              <button
                onClick={previous}
                disabled={unavailable || state.busy.includes("runs")}
              >
                {m.previous}
              </button>
            )}
            {state.nextOffset !== undefined && (
              <button
                onClick={more}
                disabled={unavailable || state.busy.includes("runs")}
              >
                {m.next}
              </button>
            )}
          </section>
        </aside>
        <ExecutionView state={state} />
      </div>
    </main>
  )
}
