import { useEffect, useRef, useState } from 'react'

type LogEntry = {
  timestamp: string
  level: string
  message: string
}

type Props = {
  logs: LogEntry[]
}

const levelColor: Record<string, string> = {
  ERROR: 'text-red-400',
  WARN: 'text-amber-400',
  INFO: 'text-blue-400',
  DEBUG: 'text-zinc-500',
  TRACE: 'text-zinc-600',
}

export function LogViewer({ logs }: Props) {
  const bottomRef = useRef<HTMLDivElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const [autoScroll, setAutoScroll] = useState(true)
  const [filter, setFilter] = useState('')
  const [levelFilter, setLevelFilter] = useState<string>('ALL')

  useEffect(() => {
    if (autoScroll && bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, [logs, autoScroll])

  const handleScroll = () => {
    if (!containerRef.current) return
    const { scrollTop, scrollHeight, clientHeight } = containerRef.current
    setAutoScroll(scrollHeight - scrollTop - clientHeight < 60)
  }

  const filtered = logs.filter(l => {
    if (levelFilter !== 'ALL' && l.level !== levelFilter) return false
    if (filter && !l.message.toLowerCase().includes(filter.toLowerCase())) return false
    return true
  })

  return (
    <div className="bg-zinc-900/50 border border-zinc-800 rounded-xl flex flex-col" style={{ height: 'calc(100vh - 300px)', minHeight: '400px' }}>
      {/* Toolbar */}
      <div className="px-4 py-3 border-b border-zinc-800 flex items-center gap-3 flex-wrap">
        <h2 className="font-semibold text-sm">Logs</h2>
        <span className="text-xs text-zinc-500">{filtered.length} entries</span>

        <div className="flex gap-1 ml-auto">
          {['ALL', 'ERROR', 'WARN', 'INFO', 'DEBUG'].map(lv => (
            <button
              key={lv}
              onClick={() => setLevelFilter(lv)}
              className={`text-xs px-2 py-1 rounded transition-colors ${
                levelFilter === lv
                  ? 'bg-zinc-700 text-white'
                  : 'bg-zinc-800/50 text-zinc-500 hover:text-zinc-300'
              }`}
            >
              {lv}
            </button>
          ))}
        </div>

        <input
          type="text"
          placeholder="Filter logs..."
          value={filter}
          onChange={e => setFilter(e.target.value)}
          className="bg-zinc-800 border border-zinc-700 rounded px-3 py-1 text-xs text-zinc-300 w-48 placeholder-zinc-600 focus:outline-none focus:border-zinc-500"
        />

        <button
          onClick={() => setAutoScroll(!autoScroll)}
          className={`text-xs px-2 py-1 rounded ${
            autoScroll ? 'bg-emerald-900/50 text-emerald-300' : 'bg-zinc-800 text-zinc-500'
          }`}
        >
          Auto-scroll {autoScroll ? 'ON' : 'OFF'}
        </button>
      </div>

      {/* Log entries */}
      <div
        ref={containerRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto font-mono text-xs p-3 space-y-0"
      >
        {filtered.length === 0 && (
          <div className="text-zinc-500 text-center py-8">
            {logs.length === 0 ? 'Waiting for logs...' : 'No logs match filter'}
          </div>
        )}
        {filtered.map((log, i) => (
          <div key={i} className="flex gap-2 py-0.5 hover:bg-zinc-800/50 px-1 rounded">
            <span className="text-zinc-600 shrink-0">{log.timestamp.split(' ')[1] || log.timestamp}</span>
            <span className={`shrink-0 w-12 text-right ${levelColor[log.level] || 'text-zinc-400'}`}>
              {log.level}
            </span>
            <span className="text-zinc-300 break-all">{log.message}</span>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  )
}
