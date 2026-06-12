import { useEffect, useRef, useState } from "react"
import { subscribeLogs } from "../api"
import type { LogLine } from "../types"

const LEVELS = ["error", "warn", "info", "debug", "trace"] as const
const MAX_LINES = 1000

/** Live tail of the daemon's logs (criterion 20). */
export function Logs() {
  const [level, setLevel] = useState<(typeof LEVELS)[number]>("info")
  const [paused, setPaused] = useState(false)
  const [lines, setLines] = useState<LogLine[]>([])
  const pausedRef = useRef(paused)
  pausedRef.current = paused
  const bottomRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    setLines([])
    const close = subscribeLogs(level, (line) => {
      if (pausedRef.current) return
      setLines((prev) => {
        const next = prev.length >= MAX_LINES ? prev.slice(-MAX_LINES + 1) : prev.slice()
        next.push(line)
        return next
      })
    })
    return close
  }, [level])

  useEffect(() => {
    if (!paused) bottomRef.current?.scrollIntoView({ behavior: "instant" as ScrollBehavior })
  }, [lines, paused])

  return (
    <div className="panel">
      <h2>Logs</h2>
      <div className="row">
        <label>
          min level{" "}
          <select value={level} onChange={(e) => setLevel(e.target.value as (typeof LEVELS)[number])}>
            {LEVELS.map((l) => (
              <option key={l} value={l}>
                {l}
              </option>
            ))}
          </select>
        </label>
        <button onClick={() => setPaused((p) => !p)}>{paused ? "resume" : "pause"}</button>
        <button onClick={() => setLines([])}>clear</button>
      </div>
      <pre className="log-tail">
        {lines.map((l, i) => (
          <div key={i} className={`log-line log-${l.level.toLowerCase()}`}>
            <span className="muted">{l.ts}</span> <strong>{l.level}</strong>{" "}
            <span className="muted">{l.target}</span> {l.message}
          </div>
        ))}
        <div ref={bottomRef} />
      </pre>
    </div>
  )
}
