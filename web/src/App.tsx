import { useEffect, useState } from "react"
import { api, getToken, setToken, validateToken } from "./api"
import { RoleContext } from "./role"
import type { Me } from "./types"
import { Approvals } from "./pages/Approvals"
import { Dashboard } from "./pages/Dashboard"
import { FlowBuilder } from "./pages/FlowBuilder"
import { GraphPage } from "./pages/Graph"
import { JobDetail } from "./pages/Job"
import { Logs } from "./pages/Logs"
import { Secrets } from "./pages/Secrets"
import { Tokens } from "./pages/Tokens"

// Hash router: #/ #/graph #/jobs/<id> #/flows #/approvals #/secrets #/logs
// #/tokens — no router dep.
function useHash(): string {
  const [hash, setHash] = useState(window.location.hash || "#/")
  useEffect(() => {
    const onChange = () => setHash(window.location.hash || "#/")
    window.addEventListener("hashchange", onChange)
    return () => window.removeEventListener("hashchange", onChange)
  }, [])
  return hash
}

/**
 * Login view (criterion 7): the token is validated against /api/v1/me BEFORE
 * it is persisted — a bad token shows an inline error and writes nothing.
 */
function Login({ onLogin }: { onLogin: (me: Me) => void }) {
  const [value, setValue] = useState("")
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)

  const submit = async () => {
    const candidate = value.trim()
    if (!candidate || busy) return
    setBusy(true)
    setError("")
    try {
      const me = await validateToken(candidate)
      setToken(candidate) // persist only after the server accepted it
      onLogin(me)
    } catch {
      setError("Invalid token — check `talon token list` or [gateway] api_token.")
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="token-gate panel">
      <h2>Talon Console</h2>
      <p className="muted">
        Sign in with a named API token (<code>talon token create</code>) or the
        legacy <code>[gateway] api_token</code> from{" "}
        <code>~/.talon/config.toml</code>.
      </p>
      {error && <p className="error">{error}</p>}
      <div className="row">
        <input
          type="password"
          value={value}
          placeholder="talon_…"
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void submit()
          }}
        />
        <button className="primary" onClick={() => void submit()} disabled={busy}>
          {busy ? "…" : "Sign in"}
        </button>
      </div>
    </div>
  )
}

export function App() {
  const hash = useHash()
  const [me, setMe] = useState<Me | null>(null)
  const [checking, setChecking] = useState(() => getToken() !== "")

  // A stored token from a previous session still gets validated — it may
  // have been revoked since.
  useEffect(() => {
    if (!checking) return
    api
      .me()
      .then(setMe)
      .catch(() => setToken(""))
      .finally(() => setChecking(false))
  }, [checking])

  if (checking) return <div className="panel muted">…</div>
  if (!me) return <Login onLogin={setMe} />

  const isAdmin = me.role === "admin"
  const jobMatch = hash.match(/^#\/jobs\/(.+)$/)
  const page = jobMatch ? (
    <JobDetail id={decodeURIComponent(jobMatch[1])} />
  ) : hash.startsWith("#/graph") ? (
    <GraphPage />
  ) : hash.startsWith("#/flows") && isAdmin ? (
    <FlowBuilder />
  ) : hash.startsWith("#/approvals") ? (
    <Approvals />
  ) : hash.startsWith("#/secrets") ? (
    <Secrets />
  ) : hash.startsWith("#/logs") ? (
    <Logs />
  ) : hash.startsWith("#/tokens") && isAdmin ? (
    <Tokens />
  ) : (
    <Dashboard />
  )

  const link = (to: string, label: string, active: boolean) => (
    <a href={to} className={active ? "active" : ""}>
      {label}
    </a>
  )

  return (
    <RoleContext.Provider value={me.role}>
      <nav>
        <span className="brand">🦅 talon</span>
        {link("#/", "Dashboard", hash === "#/" || hash === "")}
        {link("#/graph", "Graph", hash.startsWith("#/graph"))}
        {isAdmin && link("#/flows", "Flow Builder", hash.startsWith("#/flows"))}
        {link("#/approvals", "Approvals", hash.startsWith("#/approvals"))}
        {link("#/secrets", "Secrets", hash.startsWith("#/secrets"))}
        {link("#/logs", "Logs", hash.startsWith("#/logs"))}
        {isAdmin && link("#/tokens", "Tokens", hash.startsWith("#/tokens"))}
        <span style={{ flex: 1 }} />
        <span className="muted">
          {me.name} ({me.role})
        </span>
        <button
          onClick={() => {
            setToken("")
            setMe(null)
          }}
        >
          sign out
        </button>
      </nav>
      <main>{page}</main>
    </RoleContext.Provider>
  )
}
