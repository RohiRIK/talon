import { useState } from "react"
import { api } from "../api"
import type { PlannedJob } from "../types"

/// Per-job editable grant box: predicted scope rendered as a checklist.
interface EditableJob extends PlannedJob {
  grantedTools: Record<string, boolean>
  grantedBash: Record<string, boolean>
}

function toEditable(job: PlannedJob): EditableJob {
  return {
    ...job,
    grantedTools: Object.fromEntries(
      job.predicted_scope.tools.map((t) => [t, true]),
    ),
    grantedBash: Object.fromEntries(
      job.predicted_scope.bash_patterns.map((p) => [p, true]),
    ),
  }
}

export function FlowBuilder() {
  const [description, setDescription] = useState("")
  const [draft, setDraft] = useState<EditableJob[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const [created, setCreated] = useState(0)

  const plan = () => {
    setBusy(true)
    setError("")
    setCreated(0)
    api
      .planFlow(description)
      .then((resp) => setDraft(resp.jobs.map(toEditable)))
      .catch((e) => setError(String(e.message ?? e)))
      .finally(() => setBusy(false))
  }

  const commit = () => {
    setBusy(true)
    setError("")
    const jobs = draft.map((j) => ({
      key: j.key,
      name: j.name,
      schedule: j.schedule,
      prompt: j.prompt,
      deliver_to: j.deliver_to,
      tz: j.tz,
      context_from: j.context_from,
      granted_scope: {
        tools: Object.entries(j.grantedTools)
          .filter(([, on]) => on)
          .map(([t]) => t),
        bash_patterns: Object.entries(j.grantedBash)
          .filter(([, on]) => on)
          .map(([p]) => p),
      },
    }))
    api
      .commitFlow(jobs)
      .then((resp) => {
        setCreated(resp.created.length)
        setDraft([])
        setDescription("")
      })
      .catch((e) => setError(String(e.message ?? e)))
      .finally(() => setBusy(false))
  }

  const update = (i: number, patch: Partial<EditableJob>) => {
    setDraft((d) => d.map((j, idx) => (idx === i ? { ...j, ...patch } : j)))
  }

  return (
    <>
      <h2>Flow Builder</h2>
      <p className="muted">
        Describe an automation in plain language. The agent drafts a flow —
        one or more scheduled jobs, wired by <code>context_from</code> — and
        you approve the capability scope before anything is created.
      </p>
      {error && <div className="error-banner">{error}</div>}
      {created > 0 && (
        <div className="panel">
          ✅ created {created} job{created > 1 ? "s" : ""} —{" "}
          <a href="#/graph">see the graph</a>
        </div>
      )}
      <div className="panel">
        <textarea
          rows={3}
          value={description}
          placeholder='e.g. "every morning at 7, summarize my unread email, then post a digest to telegram"'
          onChange={(e) => setDescription(e.target.value)}
        />
        <div className="row" style={{ marginTop: 8 }}>
          <button
            className="primary"
            disabled={busy || !description.trim()}
            onClick={plan}
          >
            {busy ? "planning…" : "Plan flow"}
          </button>
        </div>
      </div>

      {draft.map((job, i) => (
        <div className="panel" key={job.key}>
          <div className="row">
            <strong>
              {i + 1}. {job.name}
            </strong>
            <span className="muted">({job.key})</span>
            {job.context_from.length > 0 && (
              <span className="muted">← after {job.context_from.join(", ")}</span>
            )}
          </div>
          <table>
            <tbody>
              <tr>
                <td className="muted" style={{ width: 110 }}>
                  schedule
                </td>
                <td>
                  <input
                    value={job.schedule}
                    onChange={(e) => update(i, { schedule: e.target.value })}
                  />
                </td>
              </tr>
              <tr>
                <td className="muted">prompt</td>
                <td>
                  <textarea
                    rows={2}
                    value={job.prompt}
                    onChange={(e) => update(i, { prompt: e.target.value })}
                  />
                </td>
              </tr>
              <tr>
                <td className="muted">deliver to</td>
                <td>
                  <input
                    value={job.deliver_to ?? "origin"}
                    onChange={(e) => update(i, { deliver_to: e.target.value })}
                  />
                </td>
              </tr>
            </tbody>
          </table>
          <div className="muted" style={{ marginTop: 8 }}>
            grant box — what this job may do unattended:
          </div>
          {Object.keys(job.grantedTools).length === 0 &&
            Object.keys(job.grantedBash).length === 0 && (
              <p className="muted">
                nothing predicted — every tool call will escalate to the
                Approvals inbox
              </p>
            )}
          {Object.keys(job.grantedTools).map((tool) => (
            <label className="scope-item" key={tool}>
              <input
                type="checkbox"
                checked={job.grantedTools[tool]}
                onChange={(e) =>
                  update(i, {
                    grantedTools: {
                      ...job.grantedTools,
                      [tool]: e.target.checked,
                    },
                  })
                }
              />
              {tool}
            </label>
          ))}
          {Object.keys(job.grantedBash).map((pattern) => (
            <label className="scope-item" key={pattern}>
              <input
                type="checkbox"
                checked={job.grantedBash[pattern]}
                onChange={(e) =>
                  update(i, {
                    grantedBash: {
                      ...job.grantedBash,
                      [pattern]: e.target.checked,
                    },
                  })
                }
              />
              bash: {pattern}
            </label>
          ))}
        </div>
      ))}

      {draft.length > 0 && (
        <button className="primary" disabled={busy} onClick={commit}>
          {busy ? "creating…" : `Create flow (${draft.length} job${draft.length > 1 ? "s" : ""})`}
        </button>
      )}
    </>
  )
}
