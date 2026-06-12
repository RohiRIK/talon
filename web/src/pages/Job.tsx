import { useCallback, useEffect, useState } from "react"
import { api, humanizeSince, subscribeEvents } from "../api"
import { useRole } from "../role"
import type { CronRun, JobView } from "../types"
import { jobLabel, scheduleLabel } from "../types"

function RunRow({ run }: { run: CronRun }) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <tr style={{ cursor: "pointer" }} onClick={() => setOpen(!open)}>
        <td>
          <span className={`badge ${run.status}`}>{run.status}</span>
        </td>
        <td>{run.started_at}</td>
        <td>{run.finished_at ?? "—"}</td>
        <td className="muted">{humanizeSince(run.started_at)}</td>
      </tr>
      {open && (
        <tr>
          <td colSpan={4}>
            {run.error && (
              <>
                <div className="muted">error</div>
                <pre>{run.error}</pre>
              </>
            )}
            {run.output && (
              <>
                <div className="muted">output</div>
                <pre>{run.output}</pre>
              </>
            )}
            {run.events_json && (
              <>
                <div className="muted">events</div>
                <pre>{run.events_json}</pre>
              </>
            )}
            {!run.error && !run.output && !run.events_json && (
              <p className="muted">no recorded output</p>
            )}
          </td>
        </tr>
      )}
    </>
  )
}

export function JobDetail({ id }: { id: string }) {
  const role = useRole()
  const [job, setJob] = useState<JobView | null>(null)
  const [runs, setRuns] = useState<CronRun[]>([])
  const [error, setError] = useState("")

  const refresh = useCallback(() => {
    api
      .getJob(id)
      .then(setJob)
      .catch((e) => setError(String(e.message ?? e)))
    api
      .listRuns(id, 100)
      .then(setRuns)
      .catch(() => {})
  }, [id])

  useEffect(() => {
    refresh()
    return subscribeEvents((ev) => {
      if ("job_id" in ev && ev.job_id === id) refresh()
    })
  }, [id, refresh])

  if (error) return <div className="error-banner">{error}</div>
  if (!job) return <p className="muted">loading…</p>

  return (
    <>
      <h2>
        <span className={`glyph ${job.enabled ? "success" : "neutral"}`} />
        {jobLabel(job)}
      </h2>
      <div className="panel">
        <table>
          <tbody>
            <tr>
              <td className="muted">id</td>
              <td>{job.id}</td>
            </tr>
            <tr>
              <td className="muted">prompt</td>
              <td>{job.prompt}</td>
            </tr>
            <tr>
              <td className="muted">schedule</td>
              <td>
                {scheduleLabel(job.schedule)} ({job.tz})
              </td>
            </tr>
            <tr>
              <td className="muted">deliver to</td>
              <td>{job.deliver_to}</td>
            </tr>
            <tr>
              <td className="muted">next run</td>
              <td>{job.next_run ?? "—"}</td>
            </tr>
            <tr>
              <td className="muted">runs (scheduled)</td>
              <td>
                {job.run_count}
                {job.repeat ? ` / ${job.repeat}` : ""}
              </td>
            </tr>
            {job.context_from.length > 0 && (
              <tr>
                <td className="muted">context from</td>
                <td>
                  {job.context_from.map((p) => (
                    <a key={p} href={`#/jobs/${encodeURIComponent(p)}`}>
                      {p.slice(0, 8)}{" "}
                    </a>
                  ))}
                </td>
              </tr>
            )}
            <tr>
              <td className="muted">granted scope</td>
              <td>
                {job.granted_scope.tools.length === 0 &&
                job.granted_scope.bash_patterns.length === 0 ? (
                  <span className="muted">
                    nothing pre-authorized — every tool escalates
                  </span>
                ) : (
                  <>
                    {job.granted_scope.tools.map((t) => (
                      <span key={t} className="badge" style={{ marginRight: 4 }}>
                        {t}
                      </span>
                    ))}
                    {job.granted_scope.bash_patterns.map((p) => (
                      <span key={p} className="badge" style={{ marginRight: 4 }}>
                        bash: {p}
                      </span>
                    ))}
                  </>
                )}
              </td>
            </tr>
          </tbody>
        </table>
        {role === "admin" && (
          <div className="row" style={{ marginTop: 12 }}>
            <button onClick={() => api.trigger(job.id).catch(() => {})}>
              ▶ run now
            </button>
            <button
              onClick={() =>
                api.setEnabled(job.id, !job.enabled).then(refresh)
              }
            >
              {job.enabled ? "disable" : "enable"}
            </button>
            <button
              className="danger"
              onClick={() => {
                if (confirm(`Delete job "${jobLabel(job)}"?`)) {
                  api.deleteJob(job.id).then(() => {
                    window.location.hash = "#/"
                  })
                }
              }}
            >
              delete
            </button>
          </div>
        )}
      </div>

      <h3>Run History</h3>
      {runs.length === 0 ? (
        <p className="muted">never run</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>status</th>
              <th>started</th>
              <th>finished</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {runs.map((run) => (
              <RunRow key={run.id} run={run} />
            ))}
          </tbody>
        </table>
      )}
    </>
  )
}
