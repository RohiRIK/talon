// Mirrors the serde shapes in crates/talon-gateway/src/web/.

export type RunStatus =
  | "running"
  | "success"
  | "failure"
  | "timeout"
  | "skipped"
  | "denied"

export interface GrantedScope {
  tools: string[]
  bash_patterns: string[]
}

export type CronSchedule =
  | { Cron: string }
  | { Human: string }
  | { Once: number }

export interface CronRun {
  id: string
  job_id: string
  started_at: string
  finished_at: string | null
  status: RunStatus
  output: string | null
  error: string | null
  events_json: string | null
  /** What fired the run: cron | manual | webhook | failure (handler). */
  fired_by?: string
  /** Attempt number within a retry chain; first attempt = 1. */
  attempt?: number
}

export interface JobView {
  id: string
  name: string | null
  schedule: CronSchedule
  prompt: string
  session_id: string
  deliver_to: string
  context_from: string[]
  granted_scope: GrantedScope
  enabled: boolean
  tz: string
  repeat: number | null
  run_count: number
  last_run: string | null
  last_output: string | null
  next_run: string | null
  created_at: string
  latest_run: CronRun | null
  /** Retry policy: failed attempts retried up to this many times. */
  retry_max?: number
  /** Error-handler job id, triggered when the final attempt fails. */
  on_failure?: string | null
}

export interface GraphEdge {
  from: string
  to: string
}

export interface GraphResponse {
  nodes: JobView[]
  edges: GraphEdge[]
}

export interface PlannedJob {
  key: string
  name: string
  schedule: string
  prompt: string
  deliver_to: string | null
  tz: string | null
  context_from: string[]
  predicted_scope: GrantedScope
}

export interface PlanResponse {
  jobs: PlannedJob[]
}

export interface PendingApproval {
  call_id: string
  job_id: string | null
  tool: string
  args: unknown
  requested_at: number
}

export type RunEvent =
  | { type: "run_started"; job_id: string; run_id: string }
  | { type: "run_finished"; job_id: string; run_id: string; status: RunStatus }
  | { type: "job_changed"; job_id: string }
  | {
      type: "approval_pending"
      call_id: string
      job_id: string | null
      tool: string
      args: unknown
    }
  | { type: "approval_resolved"; call_id: string; approved: boolean }

export type Role = "admin" | "viewer"

export interface Me {
  name: string
  role: Role
}

export interface TokenMeta {
  name: string
  role: Role
  created_at: string
  last_used: string | null
  revoked: boolean
}

export interface CreatedToken {
  name: string
  role: Role
  /** Shown exactly once — never retrievable again. */
  token: string
}

export interface SecretMeta {
  name: string
  provider: string
  created_at: string | null
}

export interface LogLine {
  ts: string
  level: "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE"
  target: string
  message: string
}

export function scheduleLabel(s: CronSchedule): string {
  if ("Cron" in s) return s.Cron
  if ("Human" in s) return s.Human
  return `once @${s.Once}`
}

export function jobLabel(j: JobView): string {
  return j.name ?? j.id.slice(0, 8)
}
