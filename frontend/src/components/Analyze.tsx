import { useState, useEffect } from 'react'
import { api } from '../hooks/useApi'

export function Analyze() {
  const [wallet, setWallet] = useState('')
  const [analysis, setAnalysis] = useState<any>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [leaderboard, setLeaderboard] = useState<any[]>([])
  const [btcMarkets, setBtcMarkets] = useState<any[]>([])

  useEffect(() => {
    api.getLeaderboard('CRYPTO', 'WEEK', 15)
      .then(setLeaderboard)
      .catch(() => {})
    api.getBtcMarkets()
      .then(setBtcMarkets)
      .catch(() => {})
  }, [])

  const handleAnalyze = async (addr?: string) => {
    const target = addr || wallet.trim()
    if (!target) return
    setLoading(true)
    setError('')
    setAnalysis(null)
    try {
      const data = await api.analyzeWallet(target)
      setAnalysis(data)
      setWallet(target)
    } catch (e: any) {
      setError(e.message || 'Failed to analyze wallet')
    }
    setLoading(false)
  }

  const handleTrack = async () => {
    if (!analysis) return
    await api.addWallet(analysis.wallet, `Score: ${analysis.copyability_score.toFixed(0)}`, 0, analysis.total_volume)
    alert('Wallet added to tracking!')
  }

  const scoreColor = (score: number) => {
    if (score >= 70) return 'text-emerald-400'
    if (score >= 50) return 'text-amber-400'
    return 'text-red-400'
  }

  const scoreBg = (score: number) => {
    if (score >= 70) return 'bg-emerald-500'
    if (score >= 50) return 'bg-amber-500'
    return 'bg-red-500'
  }

  return (
    <div className="space-y-6">
      {/* Search */}
      <div>
        <h2 className="text-lg font-semibold mb-3">Analyze Wallet</h2>
        <div className="flex gap-2">
          <input
            value={wallet}
            onChange={e => setWallet(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && handleAnalyze()}
            placeholder="0x... wallet address"
            className="flex-1 bg-zinc-900 border border-zinc-700 rounded-lg px-4 py-2.5 text-sm font-mono focus:border-blue-500 outline-none"
          />
          <button
            onClick={() => handleAnalyze()}
            disabled={loading}
            className="px-6 py-2.5 bg-blue-600 hover:bg-blue-700 disabled:bg-zinc-700 rounded-lg text-white text-sm font-medium"
          >
            {loading ? 'Analyzing...' : 'Analyze'}
          </button>
        </div>
        {error && <div className="mt-2 text-red-400 text-sm">{error}</div>}
      </div>

      {/* Quick pick from leaderboard */}
      <div>
        <h3 className="text-sm font-medium text-zinc-400 mb-2">Quick Pick — Top Crypto Traders (Week)</h3>
        <div className="flex flex-wrap gap-2">
          {leaderboard.filter(e => (e.vol || 0) > 0).slice(0, 10).map((e: any, i: number) => {
            const wallet = e.proxyWallet || e.proxy_wallet || ''
            return (
            <button
              key={wallet || i}
              onClick={() => handleAnalyze(wallet)}
              className="flex items-center gap-2 px-3 py-1.5 bg-zinc-900 border border-zinc-800 rounded-lg hover:border-blue-500 text-sm transition-colors"
            >
              <span className="text-zinc-500">#{i+1}</span>
              <span className="font-mono text-xs">{wallet.slice(0, 8)}...</span>
              <span className="text-emerald-400 text-xs font-mono">+${((e.pnl||0)/1000).toFixed(0)}K</span>
              <span className="text-zinc-500 text-xs">${((e.vol||0)/1000).toFixed(0)}K vol</span>
            </button>
          )})}
          {leaderboard.filter(e => e.vol > 0).length === 0 && (
            <span className="text-zinc-600 text-sm">Only showing wallets with active volume...</span>
          )}
        </div>
      </div>

      {/* Analysis result */}
      {analysis && (
        <div className="space-y-4">
          {/* Copyability score */}
          <div className="bg-zinc-900 border border-zinc-800 rounded-xl p-6">
            <div className="flex items-center justify-between mb-4">
              <div>
                <h3 className="text-lg font-semibold">Copyability Score</h3>
                <p className="font-mono text-sm text-zinc-500 mt-1">{analysis.wallet}</p>
              </div>
              <div className="text-right">
                <div className={`text-4xl font-bold font-mono ${scoreColor(analysis.copyability_score)}`}>
                  {analysis.copyability_score.toFixed(0)}
                </div>
                <div className="text-xs text-zinc-500">/ 100</div>
              </div>
            </div>
            {/* Score bar */}
            <div className="w-full bg-zinc-800 rounded-full h-2 mb-4">
              <div
                className={`h-2 rounded-full ${scoreBg(analysis.copyability_score)}`}
                style={{ width: `${analysis.copyability_score}%` }}
              />
            </div>
            {/* Reasons */}
            <div className="space-y-1">
              {analysis.copyability_reasons.map((r: string, i: number) => (
                <div key={i} className="text-sm flex items-center gap-2">
                  <span className={r.includes('-') || r.includes('harder') || r.includes('Too') || r.includes('Dust') || r.includes('Low') || r.includes('Spread')
                    ? 'text-red-400' : 'text-emerald-400'}>
                    {r.includes('-') || r.includes('harder') || r.includes('Too') || r.includes('Dust') || r.includes('Low') || r.includes('Spread')
                      ? '✗' : '✓'}
                  </span>
                  <span className="text-zinc-300">{r}</span>
                </div>
              ))}
            </div>
            {analysis.copyability_score >= 50 && (
              <button
                onClick={handleTrack}
                className="mt-4 px-5 py-2 bg-emerald-600 hover:bg-emerald-700 rounded-lg text-white text-sm font-medium"
              >
                + Add to Tracking
              </button>
            )}
          </div>

          {/* Stats grid */}
          <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
            {[
              { label: 'Last Activity', value: analysis.last_activity_ago || 'unknown', color: analysis.last_activity_ago?.includes('m ago') || analysis.last_activity_ago?.includes('s ago') ? 'text-emerald-400' : analysis.last_activity_ago?.includes('h ago') ? 'text-amber-400' : 'text-red-400' },
              { label: 'Total Trades', value: analysis.total_trades, color: 'text-blue-400' },
              { label: 'Buys / Sells', value: `${analysis.buy_count} / ${analysis.sell_count}`, color: 'text-zinc-300' },
              { label: 'Trades/Day', value: analysis.trades_per_day.toFixed(1), color: 'text-blue-400' },
              { label: 'Unique Markets', value: analysis.unique_markets, color: 'text-zinc-300' },
              { label: 'Total Volume', value: `$${(analysis.total_volume/1000).toFixed(1)}K`, color: 'text-emerald-400' },
              { label: 'Avg Trade Size', value: `$${analysis.avg_trade_size.toFixed(0)}`, color: 'text-zinc-300' },
              { label: 'Avg Price', value: analysis.avg_price.toFixed(3), color: 'text-zinc-300' },
              { label: 'Buy Ratio', value: `${((analysis.buy_count / analysis.total_trades) * 100).toFixed(0)}%`, color: 'text-amber-400' },
            ].map(c => (
              <div key={c.label} className="bg-zinc-900 border border-zinc-800 rounded-lg p-3">
                <div className="text-xs text-zinc-500 uppercase tracking-wider">{c.label}</div>
                <div className={`text-lg font-mono font-bold mt-1 ${c.color}`}>{c.value}</div>
              </div>
            ))}
          </div>

          {/* Top markets */}
          <div className="bg-zinc-900 border border-zinc-800 rounded-xl">
            <div className="px-4 py-3 border-b border-zinc-800">
              <h3 className="font-semibold text-sm">Top Markets</h3>
            </div>
            <div className="divide-y divide-zinc-800">
              {analysis.top_markets.map((m: any) => (
                <div key={m.slug} className="flex items-center justify-between px-4 py-3">
                  <div>
                    <div className="text-sm">{m.title || m.slug}</div>
                    <div className="text-xs text-zinc-500">{m.trade_count} trades</div>
                  </div>
                  <div className="text-right">
                    <div className="text-sm font-mono text-emerald-400">${m.volume.toFixed(0)}</div>
                    <div className="text-xs text-zinc-500">avg {m.avg_price.toFixed(3)}</div>
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* Recent trades */}
          <div className="bg-zinc-900 border border-zinc-800 rounded-xl">
            <div className="px-4 py-3 border-b border-zinc-800">
              <h3 className="font-semibold text-sm">Recent Trades (last 20)</h3>
            </div>
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-zinc-500 text-xs uppercase border-b border-zinc-800">
                    <th className="text-left p-3">Time</th>
                    <th className="text-left p-3">Side</th>
                    <th className="text-left p-3">Market</th>
                    <th className="text-left p-3">Outcome</th>
                    <th className="text-right p-3">Price</th>
                    <th className="text-right p-3">Value</th>
                  </tr>
                </thead>
                <tbody>
                  {analysis.recent_trades.map((t: any, i: number) => (
                    <tr key={i} className="border-b border-zinc-800/50">
                      <td className="p-3 text-zinc-400 font-mono text-xs">
                        {new Date(t.timestamp * 1000).toLocaleString('pt-BR', { month:'2-digit', day:'2-digit', hour:'2-digit', minute:'2-digit' })}
                      </td>
                      <td className={`p-3 font-medium ${t.side === 'BUY' ? 'text-emerald-400' : 'text-red-400'}`}>{t.side}</td>
                      <td className="p-3 text-zinc-300">{(t.title || t.slug || '').slice(0, 40)}</td>
                      <td className="p-3">{t.outcome}</td>
                      <td className="p-3 text-right font-mono">{t.price.toFixed(3)}</td>
                      <td className="p-3 text-right font-mono">${t.value.toFixed(2)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      )}

      {/* BTC Markets */}
      <div className="bg-zinc-900 border border-zinc-800 rounded-xl">
        <div className="px-4 py-3 border-b border-zinc-800">
          <h3 className="font-semibold text-sm">Bitcoin Short-Term Markets (Active)</h3>
        </div>
        {btcMarkets.length === 0 ? (
          <div className="text-zinc-500 text-sm text-center py-6">Loading BTC markets...</div>
        ) : (
          <div className="divide-y divide-zinc-800">
            {btcMarkets.map((m: any) => (
              <div key={m.id} className="flex items-center justify-between px-4 py-3">
                <div>
                  <div className="text-sm">{m.question}</div>
                  <div className="text-xs text-zinc-500 font-mono">{m.slug}</div>
                </div>
                <div className="text-right flex items-center gap-4">
                  {m.outcomes?.map((o: string, i: number) => (
                    <div key={o} className="text-center">
                      <div className="text-xs text-zinc-500">{o}</div>
                      <div className={`font-mono font-bold text-sm ${
                        parseFloat(m.prices?.[i] || '0') > 0.5 ? 'text-emerald-400' : 'text-zinc-400'
                      }`}>
                        {(parseFloat(m.prices?.[i] || '0') * 100).toFixed(1)}%
                      </div>
                    </div>
                  ))}
                  <div className="text-xs text-zinc-500">
                    ${parseFloat(m.liquidity || '0').toFixed(0)} liq
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
