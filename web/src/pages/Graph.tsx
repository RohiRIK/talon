import {
  Background,
  Controls,
  Handle,
  Position,
  ReactFlow,
  type NodeProps,
} from "@xyflow/react"
import { useCallback, useEffect, useState } from "react"
import { api, subscribeEvents } from "../api"
import type { FlowEdge, FlowNode } from "../graphTransform"
import { toFlow } from "../graphTransform"

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

export function GraphPage() {
  const [nodes, setNodes] = useState<FlowNode[]>([])
  const [edges, setEdges] = useState<FlowEdge[]>([])
  const [error, setError] = useState("")

  const refresh = useCallback(() => {
    api
      .graph()
      .then((g) => {
        const flow = toFlow(g)
        setNodes(flow.nodes)
        setEdges(flow.edges)
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
      <h2>Execution Graph</h2>
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
        >
          <Background />
          <Controls />
        </ReactFlow>
      </div>
      <p className="muted">
        green = last run succeeded · red = failed/timeout/denied · amber =
        running · grey = never run · edges = context_from
      </p>
    </>
  )
}
