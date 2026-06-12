import { useCallback, useEffect, useState } from "react"
import { api, subscribeEvents } from "../api"
import { useRole } from "../role"
import type { PendingApproval } from "../types"

export function Approvals() {
  const role = useRole()
  const [pending, setPending] = useState<PendingApproval[]>([])
  const [error, setError] = useState("")

  const refresh = useCallback(() => {
    api
      .listApprovals()
      .then((p) => {
        setPending(p)
        setError("")
      })
      .catch((e) => setError(String(e.message ?? e)))
  }, [])

  useEffect(() => {
    refresh()
    return subscribeEvents((ev) => {
      if (ev.type === "approval_pending" || ev.type === "approval_resolved") {
        refresh()
      }
    })
  }, [refresh])

  const resolve = (callId: string, approve: boolean) => {
    api
      .resolveApproval(callId, approve)
      .then(refresh)
      .catch((e) => setError(String(e.message ?? e)))
  }

  return (
    <>
      <h2>Approvals</h2>
      <p className="muted">
        Out-of-scope tool calls from unattended jobs wait here. Unanswered
        escalations are denied after the timeout — nothing dangerous runs
        while you are away.
      </p>
      {error && <div className="error-banner">{error}</div>}
      {pending.length === 0 ? (
        <p className="muted">nothing pending</p>
      ) : (
        pending.map((p) => (
          <div className="panel" key={p.call_id}>
            <div className="row">
              <strong>{p.tool}</strong>
              {p.job_id && (
                <a href={`#/jobs/${encodeURIComponent(p.job_id)}`}>
                  job {p.job_id.slice(0, 8)}
                </a>
              )}
              <span className="muted">
                {new Date(p.requested_at * 1000).toLocaleTimeString()}
              </span>
            </div>
            <pre>{JSON.stringify(p.args, null, 2)}</pre>
            {role === "admin" && (
              <div className="row">
                <button className="primary" onClick={() => resolve(p.call_id, true)}>
                  ✅ allow
                </button>
                <button className="danger" onClick={() => resolve(p.call_id, false)}>
                  ❌ deny
                </button>
              </div>
            )}
          </div>
        ))
      )}
    </>
  )
}
