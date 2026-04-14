type Props = {
  events: any[]
}

export function LiveFeed({ events }: Props) {
  if (!events.length) {
    return (
      <div className="text-zinc-500 text-sm text-center py-4">
        Waiting for live events...
      </div>
    )
  }

  return (
    <div className="space-y-1 max-h-64 overflow-y-auto">
      {events.map((e, i) => {
        const isDetected = e.type === 'TradeDetected'
        const isResolved = e.type === 'TradeResolved'
        const d = e.data

        if (isDetected) {
          return (
            <div key={i} className="flex items-center gap-2 text-sm bg-blue-950/30 border border-blue-900/50 rounded px-3 py-2">
              <span className="text-blue-400 font-bold">NEW</span>
              <span className="font-mono text-xs text-zinc-400">{d.whale_wallet?.slice(0, 8)}</span>
              <span>{d.side} {d.outcome}</span>
              <span className="text-zinc-500">@{d.sim_entry_price?.toFixed(3)}</span>
              <span className="text-zinc-400 ml-auto">${d.sim_cost_usdc?.toFixed(2)}</span>
            </div>
          )
        }

        if (isResolved) {
          const won = d.sim_pnl > 0
          return (
            <div key={i} className={`flex items-center gap-2 text-sm rounded px-3 py-2 border ${
              won ? 'bg-emerald-950/30 border-emerald-900/50' : 'bg-red-950/30 border-red-900/50'
            }`}>
              <span className={`font-bold ${won ? 'text-emerald-400' : 'text-red-400'}`}>
                {won ? 'WIN' : 'LOSS'}
              </span>
              <span>{d.market_title?.slice(0, 30)}</span>
              <span className={`ml-auto font-mono font-bold ${won ? 'text-emerald-400' : 'text-red-400'}`}>
                {d.sim_pnl >= 0 ? '+' : ''}${d.sim_pnl?.toFixed(2)}
              </span>
            </div>
          )
        }

        return null
      })}
    </div>
  )
}
