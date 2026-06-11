import type {
  CronRun,
  GraphResponse,
  JobView,
  PendingApproval,
  PlanResponse,
  RunEvent,
} from "./types"

const TOKEN_KEY = "talon_api_token"

export function getToken(): string {
  return localStorage.getItem(TOKEN_KEY) ?? ""
}

export function setToken(token: string): void {
  localStorage.setItem(TOKEN_KEY, token)
}

class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message)
  }
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const resp = await fetch(`/api/v1${path}`, {
    method,
    headers: {
      authorization: `Bearer ${getToken()}`,
      ...(body !== undefined ? { "content-type": "application/json" } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })
  if (resp.status === 204) return undefined as T
  const text = await resp.text()
  const json = text ? JSON.parse(text) : undefined
  if (!resp.ok) {
    throw new ApiError(resp.status, json?.error ?? `HTTP ${resp.status}`)
  }
  return json as T
}

export const api = {
  listJobs: () => request<JobView[]>("GET", "/jobs"),
  getJob: (id: string) => request<JobView>("GET", `/jobs/${id}`),
  createJob: (body: unknown) => request<JobView>("POST", "/jobs", body),
  setEnabled: (id: string, enabled: boolean) =>
    request<JobView>("PATCH", `/jobs/${id}`, { enabled }),
  deleteJob: (id: string) => request<void>("DELETE", `/jobs/${id}`),
  trigger: (id: string) =>
    request<{ status: string }>("POST", `/jobs/${id}/trigger`),
  listRuns: (id: string, limit = 50) =>
    request<CronRun[]>("GET", `/jobs/${id}/runs?limit=${limit}`),
  getRun: (id: string) => request<CronRun>("GET", `/runs/${id}`),
  graph: () => request<GraphResponse>("GET", "/graph"),
  planFlow: (description: string) =>
    request<PlanResponse>("POST", "/flows/plan", { description }),
  commitFlow: (jobs: unknown[]) =>
    request<{ created: JobView[] }>("POST", "/flows", { jobs }),
  listApprovals: () => request<PendingApproval[]>("GET", "/approvals"),
  resolveApproval: (callId: string, approve: boolean) =>
    request<unknown>("POST", `/approvals/${callId}`, { approve }),
}

/// SSE feed. EventSource cannot set headers — the server accepts ?token= on
/// this endpoint only (v1 decision; localhost-default bind).
export function subscribeEvents(onEvent: (ev: RunEvent) => void): () => void {
  const source = new EventSource(
    `/api/v1/events?token=${encodeURIComponent(getToken())}`,
  )
  source.onmessage = (msg) => {
    try {
      onEvent(JSON.parse(msg.data) as RunEvent)
    } catch {
      // Malformed frame — skip; the next snapshot fetch reconciles.
    }
  }
  return () => source.close()
}

// ── Display helpers ──────────────────────────────────────────────────────────

export function humanizeUntil(iso: string | null, now = Date.now()): string {
  if (!iso) return "—"
  const secs = Math.floor((Date.parse(iso) - now) / 1000)
  if (secs <= 0) return "overdue"
  return `in ${largestUnit(secs)}`
}

export function humanizeSince(iso: string | null, now = Date.now()): string {
  if (!iso) return "never run"
  const secs = Math.floor((now - Date.parse(iso)) / 1000)
  if (secs <= 0) return "just now"
  return `${largestUnit(secs)} ago`
}

export function largestUnit(secs: number): string {
  const MIN = 60
  const HOUR = 60 * MIN
  const DAY = 24 * HOUR
  if (secs >= DAY) return `${Math.floor(secs / DAY)}d`
  if (secs >= HOUR) return `${Math.floor(secs / HOUR)}h`
  if (secs >= MIN) return `${Math.floor(secs / MIN)}m`
  return "<1m"
}
