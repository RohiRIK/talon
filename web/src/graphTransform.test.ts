import { describe, expect, it } from "vitest"
import { toFlow, toneFor } from "./graphTransform"
import type { GraphResponse, JobView, RunStatus } from "./types"

function job(id: string, parents: string[] = [], status?: RunStatus): JobView {
  return {
    id,
    name: id,
    schedule: { Cron: "0 9 * * *" },
    prompt: "p",
    session_id: "s",
    deliver_to: "origin",
    context_from: parents,
    granted_scope: { tools: [], bash_patterns: [] },
    enabled: true,
    tz: "UTC",
    repeat: null,
    run_count: 0,
    last_run: null,
    last_output: null,
    next_run: null,
    created_at: "2026-06-10T00:00:00Z",
    latest_run: status
      ? {
          id: `run-${id}`,
          job_id: id,
          started_at: "2026-06-10T00:00:00Z",
          finished_at: null,
          status,
          output: null,
          error: null,
          events_json: null,
        }
      : null,
  }
}

describe("toneFor", () => {
  it("maps statuses to tones, never-run to neutral", () => {
    expect(toneFor(undefined)).toBe("neutral")
    expect(toneFor("success")).toBe("success")
    expect(toneFor("failure")).toBe("failure")
    expect(toneFor("timeout")).toBe("failure")
    expect(toneFor("denied")).toBe("failure")
    expect(toneFor("running")).toBe("running")
    expect(toneFor("skipped")).toBe("neutral")
  })
})

describe("toFlow", () => {
  it("layers children to the right of parents and keeps edges", () => {
    const graph: GraphResponse = {
      nodes: [job("a", [], "failure"), job("b", ["a"]), job("c", ["b"])],
      edges: [
        { from: "a", to: "b" },
        { from: "b", to: "c" },
      ],
    }
    const { nodes, edges } = toFlow(graph)
    const x = Object.fromEntries(nodes.map((n) => [n.id, n.position.x]))
    expect(x.a).toBeLessThan(x.b)
    expect(x.b).toBeLessThan(x.c)
    expect(edges).toHaveLength(2)
    expect(edges[0]).toMatchObject({ source: "a", target: "b" })
    expect(nodes.find((n) => n.id === "a")?.data.tone).toBe("failure")
    expect(nodes.find((n) => n.id === "b")?.data.tone).toBe("neutral")
  })

  it("survives cycles — every node still gets a position", () => {
    const graph: GraphResponse = {
      nodes: [job("a", ["b"]), job("b", ["a"])],
      edges: [
        { from: "a", to: "b" },
        { from: "b", to: "a" },
      ],
    }
    const { nodes } = toFlow(graph)
    expect(nodes).toHaveLength(2)
    for (const n of nodes) {
      expect(Number.isFinite(n.position.x)).toBe(true)
      expect(Number.isFinite(n.position.y)).toBe(true)
    }
  })

  it("ignores dangling context_from in layout", () => {
    const graph: GraphResponse = {
      nodes: [job("a", ["missing"])],
      edges: [],
    }
    const { nodes, edges } = toFlow(graph)
    expect(nodes[0].position.x).toBe(0)
    expect(edges).toHaveLength(0)
  })
})
