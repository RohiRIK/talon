// Pure transform: API graph response → React Flow nodes/edges.
// Kept dependency-free so it is unit-testable without rendering.

import type { GraphResponse, JobView, RunStatus } from "./types"
import { jobLabel } from "./types"

export type NodeTone = "neutral" | "running" | "success" | "failure"

export interface FlowNode {
  id: string
  position: { x: number; y: number }
  data: { label: string; tone: NodeTone; job: JobView }
}

export interface FlowEdge {
  id: string
  source: string
  target: string
  animated: boolean
}

/// Node color tone from the latest run (criterion 10: never-run → neutral).
export function toneFor(status: RunStatus | undefined): NodeTone {
  switch (status) {
    case "running":
      return "running"
    case "success":
      return "success"
    case "failure":
    case "timeout":
    case "denied":
      return "failure"
    case "skipped":
    case undefined:
      return "neutral"
  }
}

/// Layered layout: depth = longest path from a root (resolvable parents
/// only). Cycles fall back to depth 0 — every node always gets a position.
/// `savedPositions` (criterion 23, localStorage) override the computed
/// layout per node; auto-layout = call without them.
export function toFlow(
  graph: GraphResponse,
  savedPositions?: Record<string, { x: number; y: number }>,
): {
  nodes: FlowNode[]
  edges: FlowEdge[]
} {
  const ids = new Set(graph.nodes.map((n) => n.id))
  const parents = new Map<string, string[]>(
    graph.nodes.map((n) => [
      n.id,
      n.context_from.filter((p) => ids.has(p)),
    ]),
  )

  const depth = new Map<string, number>()
  const visiting = new Set<string>()
  const depthOf = (id: string): number => {
    const known = depth.get(id)
    if (known !== undefined) return known
    if (visiting.has(id)) return 0 // cycle guard
    visiting.add(id)
    const ps = parents.get(id) ?? []
    const d = ps.length === 0 ? 0 : Math.max(...ps.map(depthOf)) + 1
    visiting.delete(id)
    depth.set(id, d)
    return d
  }
  graph.nodes.forEach((n) => depthOf(n.id))

  const perDepthCount = new Map<number, number>()
  const nodes = graph.nodes.map((job) => {
    const d = depth.get(job.id) ?? 0
    const row = perDepthCount.get(d) ?? 0
    perDepthCount.set(d, row + 1)
    return {
      id: job.id,
      position: savedPositions?.[job.id] ?? { x: d * 260, y: row * 110 },
      data: {
        label: jobLabel(job),
        tone: toneFor(job.latest_run?.status),
        job,
      },
    }
  })

  const edges = graph.edges.map((e) => ({
    id: `${e.from}->${e.to}`,
    source: e.from,
    target: e.to,
    animated: false,
  }))

  return { nodes, edges }
}
