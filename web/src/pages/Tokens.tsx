import { useCallback, useEffect, useState } from "react"
import { api } from "../api"
import type { Role, TokenMeta } from "../types"

/**
 * Named API token management (criterion 4). Admin-only — App never routes a
 * viewer here, and the server 403s regardless. The raw token renders exactly
 * once, immediately after creation.
 */
export function Tokens() {
  const [tokens, setTokens] = useState<TokenMeta[] | null>(null)
  const [error, setError] = useState("")
  const [name, setName] = useState("")
  const [role, setRole] = useState<Role>("viewer")
  const [createdToken, setCreatedToken] = useState("")

  const reload = useCallback(() => {
    api
      .listTokens()
      .then((t) => {
        setTokens(t)
        setError("")
      })
      .catch((e) => setError(String(e.message ?? e)))
  }, [])

  useEffect(reload, [reload])

  const create = async () => {
    if (!name.trim()) return
    try {
      const created = await api.createToken(name.trim(), role)
      setCreatedToken(created.token)
      setName("")
      reload()
    } catch (e) {
      setError(String((e as Error).message ?? e))
    }
  }

  return (
    <div className="panel">
      <h2>API Tokens</h2>
      {error && <p className="error">{error}</p>}

      <div className="row">
        <input
          value={name}
          placeholder="name (laptop, ci, dashboard-ro …)"
          onChange={(e) => setName(e.target.value)}
        />
        <select value={role} onChange={(e) => setRole(e.target.value as Role)}>
          <option value="viewer">viewer (read-only)</option>
          <option value="admin">admin</option>
        </select>
        <button className="primary" onClick={create} disabled={!name.trim()}>
          Create
        </button>
      </div>

      {createdToken && (
        <div className="panel token-once">
          <p>
            <strong>Copy this token now — it is never shown again.</strong>
          </p>
          <code>{createdToken}</code>
          <button onClick={() => setCreatedToken("")}>dismiss</button>
        </div>
      )}

      {tokens === null ? (
        <p className="muted">Loading…</p>
      ) : tokens.length === 0 ? (
        <p className="muted">No tokens. The config.toml api_token still works (legacy admin).</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>Role</th>
              <th>Created</th>
              <th>Last used</th>
              <th>Status</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {tokens.map((t) => (
              <tr key={t.name} className={t.revoked ? "muted" : ""}>
                <td>{t.name}</td>
                <td>{t.role}</td>
                <td>{t.created_at}</td>
                <td>{t.last_used ?? "—"}</td>
                <td>{t.revoked ? "revoked" : "active"}</td>
                <td>
                  {!t.revoked && (
                    <button
                      className="danger"
                      onClick={() => {
                        if (confirm(`Revoke token ${t.name}? Clients using it get 401 immediately.`)) {
                          api.revokeToken(t.name).then(reload).catch((e) =>
                            setError(String(e.message ?? e)),
                          )
                        }
                      }}
                    >
                      revoke
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}
