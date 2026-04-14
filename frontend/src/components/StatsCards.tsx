type Stats = {
  total: number; resolved: number; open: number; skipped: number
  wins: number; losses: number; total_pnl: number; total_invested: number
  open_invested: number; win_rate: number; roi: number
  avg_slippage: number; avg_win: number; avg_loss: number
}

export function StatsCards({ stats }: { stats: Stats | null }) {
  if (!stats) return <div className="text-zinc-500 p-8 text-center">Loading stats...</div>

  const cards = [
    { label: 'Net P&L', value: `$${stats.total_pnl.toFixed(2)}`, color: stats.total_pnl >= 0 ? 'text-emerald-400' : 'text-red-400' },
    { label: 'Win Rate', value: `${stats.win_rate.toFixed(1)}%`, color: stats.win_rate >= 50 ? 'text-emerald-400' : 'text-amber-400' },
    { label: 'ROI', value: `${stats.roi.toFixed(1)}%`, color: stats.roi >= 0 ? 'text-emerald-400' : 'text-red-400' },
    { label: 'Trades', value: `${stats.resolved} resolved`, color: 'text-blue-400' },
    { label: 'Open', value: `${stats.open} ($${stats.open_invested.toFixed(0)})`, color: 'text-amber-400' },
    { label: 'W / L', value: `${stats.wins} / ${stats.losses}`, color: 'text-zinc-300' },
    { label: 'Avg Win', value: `$${stats.avg_win.toFixed(2)}`, color: 'text-emerald-400' },
    { label: 'Avg Slippage', value: `$${stats.avg_slippage.toFixed(4)}`, color: 'text-zinc-400' },
  ]

  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
      {cards.map(c => (
        <div key={c.label} className="bg-zinc-900 border border-zinc-800 rounded-lg p-4">
          <div className="text-xs text-zinc-500 uppercase tracking-wider">{c.label}</div>
          <div className={`text-xl font-mono font-bold mt-1 ${c.color}`}>{c.value}</div>
        </div>
      ))}
    </div>
  )
}
