import { useCallback, useEffect, useState } from "react"
import { api, humanizeSince, humanizeUntil, subscribeEvents } from "../api"
import { useRole } from "../role"
import type { JobView } from "../types"
import { jobLabel, scheduleLabel } from "../types"

function statusBadge(job: JobView) {
  const run = job.latest_run
  if (!run) return <span className="badge skipped">never run</span>
  return <span className={`badge ${run.status}`}>{run.status}</span>
}

export function Dashboard() {
  const role = useRole()
  const [jobs, setJobs] = useState<JobView[]>([])
  const [error, setError] = useState("")

  const refresh = useCallback(() => {
    api
      .listJobs()
      .then((j) => {
        setJobs(j)
        setError("")
      })
      .catch((e) => setError(String(e.message ?? e)))
  }, [])

  useEffect(() => {
    refresh()
    return subscribeEvents(() => refresh())
  }, [refresh])

  return (
    <>
      <h2>Cron Jobs</h2>
      {error && <div className="error-banner">{error}</div>}
      {jobs.length === 0 && !error && (
        <p className="muted">
          No cron jobs scheduled. Create one in the{" "}
          <a href="#/flows">Flow Builder</a>.
        </p>
      )}
      {jobs.length > 0 && (
        <table>
          <thead>
            <tr>
              <th></th>
              <th>name</th>
              <th>next run</th>
              <th>schedule</th>
              <th>deliver to</th>
              <th>last run</th>
              <th>status</th>
              <th>actions</th>
            </tr>
          </thead>
          <tbody>
            {jobs.map((job) => (
              <tr key={job.id}>
                <td>
                  <span
                    className={`glyph ${job.enabled ? "success" : "neutral"}`}
                    title={job.enabled ? "enabled" : "disabled"}
                  />
                </td>
                <td>
                  <a href={`#/jobs/${encodeURIComponent(job.id)}`}>
                    {jobLabel(job)}
                  </a>
                </td>
                <td>{job.enabled ? humanizeUntil(job.next_run) : "—"}</td>
                <td>{scheduleLabel(job.schedule)}</td>
                <td>{job.deliver_to}</td>
                <td className="muted">{humanizeSince(job.last_run)}</td>
                <td>{statusBadge(job)}</td>
                <td>
                  {role === "admin" && (
                    <div className="row">
                      <button
                        onClick={() =>
                          api.trigger(job.id).catch((e) => setError(String(e)))
                        }
                        title="run now (does not advance the schedule)"
                      >
                        ▶
                      </button>
                      <button
                        onClick={() =>
                          api
                            .setEnabled(job.id, !job.enabled)
                            .then(refresh)
                            .catch((e) => setError(String(e)))
                        }
                      >
                        {job.enabled ? "disable" : "enable"}
                      </button>
                      <button
                        className="danger"
                        onClick={() => {
                          if (confirm(`Delete job "${jobLabel(job)}"?`)) {
                            api
                              .deleteJob(job.id)
                              .then(refresh)
                              .catch((e) => setError(String(e)))
                          }
                        }}
                      >
                        ✕
                      </button>
                    </div>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  )
}
