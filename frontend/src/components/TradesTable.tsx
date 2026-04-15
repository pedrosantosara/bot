import { useState, useEffect } from 'react'

function Countdown({ slug, since }: { slug: string; since: string }) {
  const [remaining, setRemaining] = useState('')
  const [overdue, setOverdue] = useState(false)

  useEffect(() => {
    const update = () => {
      const parts = (slug || '').split('-')
      const ts = parts.map(Number).find(n => n > 1577836800 && n < 1893456000)
      const is15m = slug.includes('15m')
      const is4h = slug.includes('4h')
      const duration = is4h ? 14400 : is15m ? 900 : 300

      let endTime: number
      if (ts) {
        endTime = (ts + duration + 15) * 1000
      } else {
        endTime = new Date(since).getTime() + (duration + 15) * 1000
      }

      const diff = Math.floor((endTime - Date.now()) / 1000)
      if (diff <= 0) {
        setRemaining('0:00')
        setOverdue(true)
      } else {
        const m = Math.floor(diff / 60)
        const s = diff % 60
        setRemaining(`${m}:${s.toString().padStart(2, '0')}`)
        setOverdue(false)
      }
    }
    update()
    const id = setInterval(update, 1000)
    return () => clearInterval(id)
  }, [slug, since])

  if (overdue) {
    return <span className="font-mono text-xs text-zinc-500 animate-pulse">resolvendo...</span>
  }
  return <span className="font-mono text-amber-400 text-xs">{remaining}</span>
}

function TradeCard({ t }: { t: any }) {
  const time = new Date(t.detection_time).toLocaleString('pt-BR', {
    month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit'
  })
  const pnl = t.sim_pnl
  const pnlColor = pnl > 0 ? 'text-emerald-400' : pnl < 0 ? 'text-red-400' : 'text-zinc-500'
  const isOpen = t.status === 'OPEN'
  const statusColor = t.status === 'RESOLVED'
    ? (pnl > 0 ? 'bg-emerald-900/50 border-emerald-800' : 'bg-red-900/30 border-red-900')
    : isOpen ? 'bg-amber-900/30 border-amber-900'
    : 'bg-zinc-800/50 border-zinc-700'
  const title = t.market_title?.length > 50 ? t.market_title.slice(0, 47) + '...' : t.market_title

  return (
    <div className={`border rounded-lg p-3 ${statusColor}`}>
      <div className="flex items-start justify-between gap-2">
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium text-white truncate">{title}</p>
          <p className="text-xs text-zinc-400 mt-0.5">{time}</p>
        </div>
        <div className="text-right shrink-0">
          {isOpen ? (
            <div className="flex flex-col items-end gap-0.5">
              <span className="px-2 py-0.5 rounded text-xs font-medium bg-amber-900 text-amber-300">OPEN</span>
              <Countdown slug={t.market_slug} since={t.detection_time} />
            </div>
          ) : (
            <span className={`px-2 py-0.5 rounded text-xs font-medium ${
              t.status === 'RESOLVED' ? 'bg-emerald-900 text-emerald-300' : 'bg-zinc-800 text-zinc-400'
            }`}>
              {t.status?.startsWith('SKIPPED') ? 'SKIP' : t.status}
            </span>
          )}
        </div>
      </div>
      <div className="flex items-center gap-3 mt-2 text-xs">
        <span className={`font-semibold px-1.5 py-0.5 rounded ${
          t.outcome === 'Yes' || t.outcome === 'Up' ? 'bg-emerald-900/50 text-emerald-300' : 'bg-red-900/50 text-red-300'
        }`}>{t.outcome}</span>
        <span className="text-zinc-500">@{t.sim_entry_price?.toFixed(2)}</span>
        <span className="font-mono text-zinc-300">${t.sim_cost_usdc?.toFixed(2)}</span>
        {pnl != null && (
          <span className={`font-mono font-bold ml-auto ${pnlColor}`}>
            {pnl >= 0 ? '+' : ''}${pnl.toFixed(2)}
          </span>
        )}
      </div>
    </div>
  )
}

export function TradesTable({ trades }: { trades: any[] }) {
  if (!trades.length) {
    return <div className="text-zinc-500 p-8 text-center text-sm">No trades yet.</div>
  }

  return (
    <>
      {/* Mobile: card layout */}
      <div className="sm:hidden space-y-2 p-2">
        {trades.map((t: any) => <TradeCard key={t.id} t={t} />)}
      </div>

      {/* Desktop: table layout */}
      <div className="hidden sm:block overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="text-zinc-500 text-xs uppercase border-b border-zinc-800">
              <th className="text-left p-3">Time</th>
              <th className="text-left p-3">Market</th>
              <th className="text-left p-3">Outcome</th>
              <th className="text-right p-3">Price</th>
              <th className="text-right p-3">Cost</th>
              <th className="text-right p-3">P&L</th>
              <th className="text-center p-3">Status</th>
            </tr>
          </thead>
          <tbody>
            {trades.map((t: any) => {
              const time = new Date(t.detection_time).toLocaleString('pt-BR', {
                month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit'
              })
              const title = t.market_title?.length > 40 ? t.market_title.slice(0, 37) + '...' : t.market_title
              const pnl = t.sim_pnl
              const pnlColor = pnl > 0 ? 'text-emerald-400' : pnl < 0 ? 'text-red-400' : 'text-zinc-500'
              const isOpen = t.status === 'OPEN'
              const statusColor = t.status === 'RESOLVED' ? 'bg-emerald-900 text-emerald-300'
                : isOpen ? 'bg-amber-900 text-amber-300'
                : 'bg-zinc-800 text-zinc-400'

              return (
                <tr key={t.id} className="border-b border-zinc-800/50 hover:bg-zinc-900/50">
                  <td className="p-3 text-zinc-400 font-mono text-xs">{time}</td>
                  <td className="p-3">{title}</td>
                  <td className="p-3 font-medium">{t.outcome}</td>
                  <td className="p-3 text-right font-mono text-xs">{t.sim_entry_price?.toFixed(3)}</td>
                  <td className="p-3 text-right font-mono">${t.sim_cost_usdc?.toFixed(2)}</td>
                  <td className={`p-3 text-right font-mono font-bold ${pnlColor}`}>
                    {pnl != null ? `${pnl >= 0 ? '+' : ''}$${pnl.toFixed(2)}` : '-'}
                  </td>
                  <td className="p-3 text-center">
                    {isOpen ? (
                      <div className="flex flex-col items-center gap-0.5">
                        <span className={`px-2 py-0.5 rounded text-xs font-medium ${statusColor}`}>OPEN</span>
                        <Countdown slug={t.market_slug} since={t.detection_time} />
                      </div>
                    ) : (
                      <span className={`px-2 py-0.5 rounded text-xs font-medium ${statusColor}`}>
                        {t.status?.startsWith('SKIPPED') ? 'SKIP' : t.status}
                      </span>
                    )}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </>
  )
}
