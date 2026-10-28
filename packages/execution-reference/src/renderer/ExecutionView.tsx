import { useState, type ChangeEvent, type FormEvent } from "react"
import { operator, type OperatorState } from "./model.js"
import { QuestionForm, PermissionForm } from "./RequestForms.js"
import { messages as m } from "./messages.js"
export function ExecutionView({ state }: { state: OperatorState }) {
  const [instruction, setInstruction] = useState("")
  const [agentId, setAgentId] = useState("")
  const run = state.run
  if (!run)
    return (
      <section>
        <h2>{m.execution}</h2>
        <p>{m.noExecution}</p>
      </section>
    )
  const unavailable = state.host.state !== "ready"
  const terminal = ["completed", "failed", "interrupted"].includes(run.status)
  const selectedAgent = run.agents.some((agent) => agent.id === agentId)
    ? agentId
    : run.rootAgentId
  const target = run.agents.find((agent) => agent.id === selectedAgent)
  const targetTerminal =
    target && ["completed", "failed", "interrupted"].includes(target.status)
  const canContinue =
    !unavailable &&
    state.probe?.available &&
    !state.busy.includes("continue") &&
    (terminal || !targetTerminal)
  function changeInstruction(event: ChangeEvent<HTMLTextAreaElement>) {
    setInstruction(event.target.value)
  }
  function changeAgent(event: ChangeEvent<HTMLSelectElement>) {
    setAgentId(event.target.value)
  }
  function submitInstruction(event: FormEvent) {
    event.preventDefault()
    void operator.continue(selectedAgent, instruction)
  }
  function stop() {
    void operator.stop()
  }
  function refresh() {
    if (run) void operator.selectRun(run.id)
  }
  function history() {
    void operator.history(false)
  }
  function moreEvents() {
    void operator.history(true)
  }
  const agents = run.agents.map((agent) => (
    <li key={agent.id}>
      <code>{agent.id}</code>
      <span>{agent.status}</span>
      {agent.parentId && <small>↳ {agent.parentId}</small>}
    </li>
  ))
  const agentOptions = run.agents.map((agent) => (
    <option key={agent.id} value={agent.id}>
      {agent.id} · {agent.status}
    </option>
  ))
  const completed = run.messages.map((message) => (
    <article className="message" key={message.id}>
      <p className="meta">{message.agentId}</p>
      <pre>{message.text}</pre>
    </article>
  ))
  const live = Object.entries(state.deltas).map(([id, message]) => (
    <article key={id} className="message">
      <p className="meta">{message.agentId}</p>
      <pre>{message.text}</pre>
    </article>
  ))
  const questions = run.questions.map((question) => (
    <QuestionForm
      key={question.id}
      runId={run.id}
      question={question}
      disabled={
        unavailable ||
        terminal ||
        state.busy.includes(`question-${question.id}`)
      }
    />
  ))
  const permissions = run.permissions.map((request) => (
    <PermissionForm
      key={request.id}
      runId={run.id}
      request={request}
      disabled={
        unavailable ||
        terminal ||
        state.busy.includes(`permission-${request.id}`)
      }
    />
  ))
  const events = state.history.map((event) => (
    <li key={event.id}>
      <time>{event.at}</time> · {event.type}
    </li>
  ))
  const stopDisabled =
    ["starting", "closing", "closed"].includes(state.host.state) ||
    terminal ||
    state.busy.includes("stop")
  return (
    <section aria-labelledby="execution-title">
      <header className="section-header">
        <h2 id="execution-title">{m.execution}</h2>
        <div className="actions">
          <button onClick={refresh}>{m.retry}</button>
          <button onClick={stop} disabled={stopDisabled}>
            {m.stop}
          </button>
        </div>
      </header>
      {unavailable && <p role="alert">{m.cleanupUnknown}</p>}
      <p>
        <strong>{run.status}</strong> · {m.version} {run.version}
      </p>
      <p>{run.task}</p>
      <p className="meta">
        {run.workspacePath} · {run.providerId}
      </p>
      {run.reason && <p role="status">{run.reason}</p>}
      {run.continuationOf && (
        <p>
          {m.linkedFrom}: <code>{run.continuationOf.runId}</code>
        </p>
      )}
      <h3>{m.agents}</h3>
      <ul className="agents">{agents}</ul>
      <h3>{m.output}</h3>
      <p className="meta">{m.plainOutput}</p>
      <h4>{m.transcript}</h4>
      {completed.length ? completed : <p>{m.noOutput}</p>}
      {live.length > 0 && (
        <>
          <h4>{m.live}</h4>
          {live}
        </>
      )}
      <h3>{m.questions}</h3>
      {questions.length ? questions : <p>{m.noRequests}</p>}
      <h3>{m.permissions}</h3>
      {permissions.length ? permissions : <p>{m.noRequests}</p>}
      <form onSubmit={submitInstruction}>
        <h3>{terminal ? m.continue : m.steer}</h3>
        <label>
          {m.target}
          <select value={selectedAgent} onChange={changeAgent}>
            {agentOptions}
          </select>
        </label>
        <label>
          {m.instruction}
          <textarea
            value={instruction}
            onChange={changeInstruction}
            required
            maxLength={65536}
          />
        </label>
        <button type="submit" disabled={!canContinue || !instruction.trim()}>
          {terminal ? m.continue : m.steer}
        </button>
      </form>
      <h3>{m.events}</h3>
      <button onClick={history}>{m.loadEvents}</button>
      <ol className="events">{events}</ol>
      {state.nextSequence !== undefined && (
        <button onClick={moreEvents}>{m.more}</button>
      )}
    </section>
  )
}
