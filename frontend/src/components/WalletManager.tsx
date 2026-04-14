import { useState, useEffect } from 'react'
import { api } from '../hooks/useApi'

type AnalysisCache = Record<string, any>

export function WalletManager({ onAnalyze }: { onAnalyze?: (wallet: string) => void }) {
  const [wallets, setWallets] = useState<any[]>([])
  const [newAddr, setNewAddr] = useState('')
  const [newLabel, setNewLabel] = useState('')
  const [leaderboard, setLeaderboard] = useState<any[]>([])
  const [showLB, setShowLB] = useState(false)
  const [lbFilter, setLbFilter] = useState<'all' | 'active' | 'high_pnl'>('active')
  const [lbCategory, setLbCategory] = useState('CRYPTO')
  const [lbPeriod, setLbPeriod] = useState('WEEK')
  const [lbLoading, setLbLoading] = useState(false)
  const [analysisCache, setAnalysisCache] = useState<AnalysisCache>({})
  const [analyzingWallet, setAnalyzingWallet] = useState<string | null>(null)

  const load = async () => {
    const w = await api.getWallets()
    setWallets(w)
  }

  useEffect(() => { load() }, [])

  const handleAdd = async () => {
    if (!newAddr.trim()) return
    await api.addWallet(newAddr.trim(), newLabel.trim())
    setNewAddr('')
    setNewLabel('')
    load()
  }

  const handleDiscover = async () => {
    setShowLB(true)
    await fetchLeaderboard(lbCategory, lbPeriod)
  }

  const fetchLeaderboard = async (category: string, period: string) => {
    setLbLoading(true)
    try {
      const lb = await api.getLeaderboard(category, period, 30)
      setLeaderboard(lb)
    } catch { /* ignore */ }
    setLbLoading(false)
  }

  const handleAddFromLB = async (entry: any) => {
    const wallet = entry.proxyWallet || entry.proxy_wallet || ''
    const label = (entry.xUsername || entry.x_username)
      ? `@${entry.xUsername || entry.x_username}`
      : (entry.userName || entry.user_name || '').slice(0, 20)
    await api.addWallet(wallet, label, entry.pnl || 0, entry.vol || 0)
    load()
  }

  const handleQuickAnalyze = async (wallet: string) => {
    if (analysisCache[wallet]) return
    setAnalyzingWallet(wallet)
    try {
      const data = await api.analyzeWallet(wallet)
      setAnalysisCache(prev => ({ ...prev, [wallet]: data }))
    } catch {
      setAnalysisCache(prev => ({ ...prev, [wallet]: { error: true } }))
    }
    setAnalyzingWallet(null)
  }

  const w = (e: any) => e.proxyWallet || e.proxy_wallet || ''
  const vol = (e: any) => e.vol || 0
  const pnl = (e: any) => e.pnl || 0
  const name = (e: any) => (e.xUsername || e.x_username) ? `@${e.xUsername || e.x_username}` : (e.userName || e.user_name || '').slice(0, 16)
  const isActive = (e: any) => vol(e) > 0

  const filtered = leaderboard.filter(e => {
    if (lbFilter === 'active') return isActive(e)
    if (lbFilter === 'high_pnl') return pnl(e) > 10000
    return true
  })

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">Tracked Wallets</h2>
        <button onClick={handleDiscover} className="text-sm px-3 py-1.5 bg-blue-600 hover:bg-blue-700 rounded-lg text-white">
          Discover from Leaderboard
        </button>
      </div>

      {/* Add wallet form */}
      <div className="flex gap-2">
        <input
          value={newAddr} onChange={e => setNewAddr(e.target.value)}
          placeholder="0x... wallet address"
          className="flex-1 bg-zinc-900 border border-zinc-700 rounded-lg px-3 py-2 text-sm font-mono focus:border-blue-500 outline-none"
        />
        <input
          value={newLabel} onChange={e => setNewLabel(e.target.value)}
          placeholder="Label (optional)"
          className="w-40 bg-zinc-900 border border-zinc-700 rounded-lg px-3 py-2 text-sm focus:border-blue-500 outline-none"
        />
        <button onClick={handleAdd} className="px-4 py-2 bg-emerald-600 hover:bg-emerald-700 rounded-lg text-white text-sm font-medium">
          Add
        </button>
      </div>

      {/* Wallet list */}
      <div className="space-y-1">
        {wallets.map(wal => (
          <div key={wal.id} className="flex items-center justify-between bg-zinc-900 border border-zinc-800 rounded-lg px-4 py-3">
            <div className="flex items-center gap-3">
              <div className={`w-2 h-2 rounded-full ${wal.enabled ? 'bg-emerald-400' : 'bg-zinc-600'}`} />
              <span className="font-mono text-sm">{wal.address.slice(0, 10)}...{wal.address.slice(-4)}</span>
              {wal.label && <span className="text-zinc-500 text-sm">{wal.label}</span>}
              <span className="text-emerald-400 text-sm font-mono">+${wal.pnl?.toFixed(0)}</span>
            </div>
            <div className="flex gap-2">
              {onAnalyze && (
                <button
                  onClick={() => onAnalyze(wal.address)}
                  className="text-xs px-3 py-1 rounded bg-blue-900 text-blue-300 hover:bg-blue-800"
                >
                  Analyze
                </button>
              )}
              <button
                onClick={async () => { await api.toggleWallet(wal.id, !wal.enabled); load() }}
                className={`text-xs px-3 py-1 rounded ${wal.enabled ? 'bg-zinc-700 text-zinc-300' : 'bg-emerald-800 text-emerald-300'}`}
              >
                {wal.enabled ? 'Disable' : 'Enable'}
              </button>
              <button
                onClick={async () => { await api.deleteWallet(wal.id); load() }}
                className="text-xs px-3 py-1 rounded bg-red-900 text-red-300 hover:bg-red-800"
              >
                Remove
              </button>
            </div>
          </div>
        ))}
        {wallets.length === 0 && (
          <div className="text-zinc-500 text-center py-6">No wallets tracked. Add a wallet or discover from leaderboard.</div>
        )}
      </div>

      {/* Leaderboard modal */}
      {showLB && (
        <div className="fixed inset-0 bg-black/70 flex items-center justify-center z-50 p-4" onClick={() => setShowLB(false)}>
          <div className="bg-zinc-900 border border-zinc-700 rounded-xl max-w-4xl w-full max-h-[85vh] overflow-hidden flex flex-col" onClick={e => e.stopPropagation()}>
            {/* Header */}
            <div className="p-4 border-b border-zinc-800">
              <div className="flex items-center justify-between mb-3">
                <h3 className="font-semibold text-lg">Polymarket Leaderboard</h3>
                <button onClick={() => setShowLB(false)} className="text-zinc-500 hover:text-white text-xl leading-none">&times;</button>
              </div>

              {/* Filters row */}
              <div className="flex flex-wrap items-center gap-3">
                {/* Category */}
                <div className="flex bg-zinc-800 rounded-lg p-0.5">
                  {['CRYPTO', 'SPORTS', 'POLITICS', 'FINANCE'].map(c => (
                    <button
                      key={c}
                      onClick={() => { setLbCategory(c); fetchLeaderboard(c, lbPeriod) }}
                      className={`px-3 py-1 rounded-md text-xs font-medium transition-colors ${lbCategory === c ? 'bg-blue-600 text-white' : 'text-zinc-400 hover:text-zinc-200'}`}
                    >
                      {c}
                    </button>
                  ))}
                </div>

                {/* Period */}
                <div className="flex bg-zinc-800 rounded-lg p-0.5">
                  {[['DAY', '24h'], ['WEEK', '7d'], ['MONTH', '30d'], ['ALL', 'All']].map(([val, label]) => (
                    <button
                      key={val}
                      onClick={() => { setLbPeriod(val); fetchLeaderboard(lbCategory, val) }}
                      className={`px-3 py-1 rounded-md text-xs font-medium transition-colors ${lbPeriod === val ? 'bg-blue-600 text-white' : 'text-zinc-400 hover:text-zinc-200'}`}
                    >
                      {label}
                    </button>
                  ))}
                </div>

                {/* Activity filter */}
                <div className="flex bg-zinc-800 rounded-lg p-0.5">
                  {([['active', 'Active Only'], ['high_pnl', 'P&L > $10K'], ['all', 'All']] as const).map(([val, label]) => (
                    <button
                      key={val}
                      onClick={() => setLbFilter(val)}
                      className={`px-3 py-1 rounded-md text-xs font-medium transition-colors ${lbFilter === val ? 'bg-amber-600 text-white' : 'text-zinc-400 hover:text-zinc-200'}`}
                    >
                      {label}
                    </button>
                  ))}
                </div>

                <span className="text-xs text-zinc-500 ml-auto">
                  {filtered.length} / {leaderboard.length} wallets
                </span>
              </div>
            </div>

            {/* Table */}
            <div className="overflow-y-auto flex-1">
              {lbLoading ? (
                <div className="text-zinc-500 text-center py-12">Loading...</div>
              ) : (
                <table className="w-full text-sm">
                  <thead className="sticky top-0 bg-zinc-900">
                    <tr className="text-zinc-500 text-xs uppercase border-b border-zinc-800">
                      <th className="text-left p-3 w-8">#</th>
                      <th className="text-left p-3">Wallet</th>
                      <th className="text-left p-3">Name</th>
                      <th className="text-right p-3">P&L</th>
                      <th className="text-right p-3">Volume</th>
                      <th className="text-center p-3">Status</th>
                      <th className="text-center p-3">Score</th>
                      <th className="text-right p-3">Actions</th>
                    </tr>
                  </thead>
                  <tbody>
                    {filtered.map((e: any, i: number) => {
                      const wallet = w(e)
                      const active = isActive(e)
                      const cached = analysisCache[wallet]
                      const isAnalyzing = analyzingWallet === wallet

                      return (
                        <tr key={wallet || i} className="border-b border-zinc-800/50 hover:bg-zinc-800/30">
                          <td className="p-3 text-zinc-500">{i + 1}</td>
                          <td className="p-3 font-mono text-xs">
                            {wallet.slice(0, 6)}...{wallet.slice(-4)}
                          </td>
                          <td className="p-3 text-zinc-400 text-xs">{name(e) || '-'}</td>
                          <td className="p-3 text-right font-mono">
                            <span className="text-emerald-400">+${(pnl(e) / 1000).toFixed(1)}K</span>
                          </td>
                          <td className="p-3 text-right font-mono">
                            {vol(e) > 0
                              ? <span className="text-blue-400">${(vol(e) / 1000).toFixed(1)}K</span>
                              : <span className="text-zinc-600">$0</span>
                            }
                          </td>
                          <td className="p-3 text-center">
                            {active
                              ? <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded bg-emerald-900/50 text-emerald-300">
                                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" /> Active
                                </span>
                              : <span className="text-xs px-2 py-0.5 rounded bg-zinc-800 text-zinc-500">Inactive</span>
                            }
                          </td>
                          <td className="p-3 text-center">
                            {cached && !cached.error ? (
                              <span className={`font-mono font-bold text-sm ${
                                cached.copyability_score >= 70 ? 'text-emerald-400' :
                                cached.copyability_score >= 50 ? 'text-amber-400' : 'text-red-400'
                              }`}>
                                {cached.copyability_score.toFixed(0)}
                              </span>
                            ) : cached?.error ? (
                              <span className="text-zinc-600 text-xs">-</span>
                            ) : (
                              <button
                                onClick={() => handleQuickAnalyze(wallet)}
                                disabled={isAnalyzing}
                                className="text-xs text-blue-400 hover:text-blue-300 disabled:text-zinc-600"
                              >
                                {isAnalyzing ? '...' : 'Check'}
                              </button>
                            )}
                          </td>
                          <td className="p-3 text-right">
                            <div className="flex items-center justify-end gap-1.5">
                              {onAnalyze && (
                                <button
                                  onClick={() => { setShowLB(false); onAnalyze(wallet) }}
                                  className="text-xs px-2.5 py-1 rounded bg-zinc-800 text-zinc-300 hover:bg-zinc-700"
                                >
                                  Analyze
                                </button>
                              )}
                              <button
                                onClick={() => handleAddFromLB(e)}
                                className="text-xs px-2.5 py-1 rounded bg-blue-600 hover:bg-blue-700 text-white"
                              >
                                + Track
                              </button>
                            </div>
                          </td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              )}
              {!lbLoading && filtered.length === 0 && (
                <div className="text-zinc-500 text-center py-8">
                  No wallets match the current filter. Try "All" or a different period.
                </div>
              )}
            </div>

            {/* Expanded analysis details */}
            {Object.entries(analysisCache).filter(([_, v]) => v && !v.error && v.top_markets).length > 0 && (
              <div className="border-t border-zinc-800 p-4 max-h-48 overflow-y-auto">
                <h4 className="text-xs uppercase text-zinc-500 mb-2">Analyzed Wallets — Top Markets</h4>
                <div className="space-y-2">
                  {Object.entries(analysisCache)
                    .filter(([_, v]) => v && !v.error && v.top_markets)
                    .map(([addr, data]) => (
                      <div key={addr} className="flex items-start gap-3 text-xs">
                        <span className="font-mono text-zinc-400 w-24 shrink-0">{addr.slice(0, 8)}...</span>
                        <div className="flex flex-wrap gap-1.5">
                          <span className={`font-bold ${
                            data.copyability_score >= 70 ? 'text-emerald-400' :
                            data.copyability_score >= 50 ? 'text-amber-400' : 'text-red-400'
                          }`}>
                            Score: {data.copyability_score.toFixed(0)}
                          </span>
                          <span className="text-zinc-600">|</span>
                          <span className={data.last_activity_ago?.includes('m ago') || data.last_activity_ago?.includes('s ago') ? 'text-emerald-400' : 'text-zinc-400'}>
                            {data.last_activity_ago || '?'}
                          </span>
                          <span className="text-zinc-600">|</span>
                          <span className="text-zinc-400">{data.trades_per_day.toFixed(1)}/day</span>
                          <span className="text-zinc-600">|</span>
                          <span className="text-zinc-400">{data.buy_count}B/{data.sell_count}S</span>
                          <span className="text-zinc-600">|</span>
                          {data.top_markets.slice(0, 3).map((m: any) => (
                            <span key={m.slug} className="px-1.5 py-0.5 bg-zinc-800 rounded text-zinc-300">
                              {(m.title || m.slug).slice(0, 25)}
                            </span>
                          ))}
                        </div>
                      </div>
                    ))}
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
