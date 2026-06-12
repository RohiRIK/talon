import { useCallback, useEffect, useState } from "react"
import { api } from "../api"
import type { SecretMeta } from "../types"
import { useRole } from "../role"

/**
 * Builtin vault management (criterion 15). Values travel one way: the create
 * form posts them, then clears local state — nothing in this page can render
 * a stored value because the API never returns one.
 */
export function Secrets() {
  const role = useRole()
  const [secrets, setSecrets] = useState<SecretMeta[] | null>(null)
  const [error, setError] = useState("")
  const [name, setName] = useState("")
  const [value, setValue] = useState("")
  const [savedRef, setSavedRef] = useState("")

  const reload = useCallback(() => {
    api
      .listSecrets()
      .then((s) => {
        setSecrets(s)
        setError("")
      })
      .catch((e) => setError(String(e.message ?? e)))
  }, [])

  useEffect(reload, [reload])

  const create = async () => {
    if (!name.trim() || !value) return
    try {
      const created = await api.createSecret(name.trim(), value)
      setSavedRef(created.reference)
      setName("")
      setValue("") // criterion 15: no value echo after save
      reload()
    } catch (e) {
      setError(String((e as Error).message ?? e))
    }
  }

  return (
    <div className="panel">
      <h2>Secrets</h2>
      <p className="muted">
        Encrypted in the builtin vault. Reference from a job prompt as{" "}
        <code>{"{{secret:NAME}}"}</code> — values are resolved at dispatch and
        never stored, logged, or shown again.
      </p>
      {error && <p className="error">{error}</p>}

      {role === "admin" && (
        <div className="row">
          <input
            value={name}
            placeholder="NAME (A-Z, 0-9, _ . -)"
            onChange={(e) => setName(e.target.value)}
          />
          <input
            type="password"
            value={value}
            placeholder="value (write-only)"
            onChange={(e) => setValue(e.target.value)}
          />
          <button className="primary" onClick={create} disabled={!name.trim() || !value}>
            Store
          </button>
        </div>
      )}
      {savedRef && (
        <p className="muted">
          Stored. Use it as <code>{savedRef}</code>
        </p>
      )}

      {secrets === null ? (
        <p className="muted">Loading…</p>
      ) : secrets.length === 0 ? (
        <p className="muted">No secrets stored.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>Provider</th>
              <th>Created</th>
              {role === "admin" && <th />}
            </tr>
          </thead>
          <tbody>
            {secrets.map((s) => (
              <tr key={s.name}>
                <td>
                  <code>{s.name}</code>
                </td>
                <td>{s.provider}</td>
                <td>{s.created_at ?? "—"}</td>
                {role === "admin" && (
                  <td>
                    <button
                      className="danger"
                      onClick={() => {
                        if (confirm(`Delete secret ${s.name}? Jobs referencing it will fail.`)) {
                          api.deleteSecret(s.name).then(reload).catch((e) =>
                            setError(String(e.message ?? e)),
                          )
                        }
                      }}
                    >
                      delete
                    </button>
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}
