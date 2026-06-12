import {
  Background,
  Controls,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  type Connection,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react"
import { useCallback, useEffect, useState } from "react"
import { api, subscribeEvents } from "../api"
import type { FlowEdge, FlowNode } from "../graphTransform"
import { toFlow } from "../graphTransform"
import { useRole } from "../role"

function JobNode({ data }: NodeProps) {
  const d = data as FlowNode["data"]
  return (
    <div className={`flow-node ${d.tone}`}>
      <Handle type="target" position={Position.Left} />
      <a href={`#/jobs/${encodeURIComponent(d.job.id)}`}>{d.label}</a>
      <Handle type="source" position={Position.Right} />
    </div>
  )
}

const nodeTypes = { job: JobNode }
const POSITIONS_KEY = "talon_graph_positions"

type Positions = Record<string, { x: number; y: number }>

function loadPositions(): Positions {
  try {
    return JSON.parse(localStorage.getItem(POSITIONS_KEY) ?? "{}") as Positions
  } catch {
    return {}
  }
}

function savePositions(p: Positions) {
  localStorage.setItem(POSITIONS_KEY, JSON.stringify(p))
}

/**
 * Graph v2 (criteria 21–24): the DAG is an editor — dragging a connection
 * between job nodes adds a `context_from` dependency, deleting an edge
 * removes it. Every edit goes through `PATCH /jobs/{id}`; the server
 * re-validates (cycles → 422) and the UI reverts by refetching. Node
 * positions persist in localStorage; auto-layout clears them.
 */
export function GraphPage() {
  const role = useRole()
  const [nodes, setNodes] = useState<FlowNode[]>([])
  const [edges, setEdges] = useState<FlowEdge[]>([])
  const [error, setError] = useState("")

  const refresh = useCallback(() => {
    api
      .graph()
      .then((g) => {
        const flow = toFlow(g, loadPositions())
        setNodes(flow.nodes)
        setEdges(flow.edges)
      })
      .catch((e) => setError(String(e.message ?? e)))
  }, [])

  useEffect(() => {
    refresh()
    return subscribeEvents(() => refresh())
  }, [refresh])

  /** Current deps of a job, derived from the rendered edges. */
  const depsOf = useCallback(
    (jobId: string) => edges.filter((e) => e.target === jobId).map((e) => e.source),
    [edges],
  )

  const onConnect = useCallback(
    (conn: Connection) => {
      if (!conn.source || !conn.target || conn.source === conn.target) return
      const next = [...new Set([...depsOf(conn.target), conn.source])]
      // Optimistic add; the server is authoritative — on 422 we refetch,
      // which reverts the edge and surfaces the reason.
      setEdges((prev) => [
        ...prev,
        {
          id: `${conn.source}->${conn.target}`,
          source: conn.source,
          target: conn.target,
          animated: false,
        },
      ])
      api
        .setContextFrom(conn.target, next)
        .then(() => setError(""))
        .catch((e) => {
          setError(String(e.message ?? e))
          refresh()
        })
    },
    [depsOf, refresh],
  )

  const onEdgesDelete = useCallback(
    (deleted: Edge[]) => {
      for (const edge of deleted) {
        const next = depsOf(edge.target).filter((d) => d !== edge.source)
        api
          .setContextFrom(edge.target, next)
          .then(() => setError(""))
          .catch((e) => {
            setError(String(e.message ?? e))
            refresh()
          })
      }
    },
    [depsOf, refresh],
  )

  const onNodeDragStop = useCallback((_: unknown, node: Node) => {
    const positions = loadPositions()
    positions[node.id] = { x: node.position.x, y: node.position.y }
    savePositions(positions)
    setNodes((prev) =>
      prev.map((n) => (n.id === node.id ? { ...n, position: node.position } : n)),
    )
  }, [])

  const autoLayout = () => {
    localStorage.removeItem(POSITIONS_KEY)
    refresh()
  }

  const editable = role === "admin"

  return (
    <>
      <div className="row" style={{ alignItems: "baseline" }}>
        <h2 style={{ flex: 1 }}>Execution Graph</h2>
        <button onClick={autoLayout} title="recompute the layered layout">
          auto-layout
        </button>
      </div>
      {error && <div className="error-banner">{error}</div>}
      <div
        style={{
          height: "70vh",
          border: "1px solid var(--border)",
          borderRadius: 8,
        }}
      >
        <ReactFlow
          nodes={nodes.map((n) => ({ ...n, type: "job" }))}
          edges={edges}
          nodeTypes={nodeTypes}
          fitView
          colorMode="dark"
          proOptions={{ hideAttribution: true }}
          nodesConnectable={editable}
          edgesFocusable={editable}
          elementsSelectable
          onConnect={editable ? onConnect : undefined}
          onEdgesDelete={editable ? onEdgesDelete : undefined}
          onNodeDragStop={onNodeDragStop}
          deleteKeyCode={editable ? ["Backspace", "Delete"] : null}
        >
          <Background />
          <Controls />
          <MiniMap pannable zoomable />
        </ReactFlow>
      </div>
      <p className="muted">
        green = last run succeeded · red = failed/timeout/denied · amber =
        running · grey = never run · edges = context_from
        {editable && " · drag node→node to add a dependency, select an edge and press Delete to remove it"}
      </p>
    </>
  )
}
