import { useState, useEffect, useCallback } from 'react'
import { StatsCards } from './components/StatsCards'
import { TradesTable } from './components/TradesTable'
import { BotControl } from './components/BotControl'
import { LiveFeed } from './components/LiveFeed'
import { Settings } from './components/Settings'
import { Analyze } from './components/Analyze'
import { LogViewer } from './components/LogViewer'
import { useWebSocket } from './hooks/useWebSocket'
import { api } from './hooks/useApi'

type Tab = 'dashboard' | 'analyze' | 'history' | 'logs' | 'settings'

export default function App() {
  const [tab, setTab] = useState<Tab>('dashboard')
  const [stats, setStats] = useState<any>(null)
  const [balance, setBalance] = useState<any>(null)
  const [trades, setTrades] = useState<any[]>([])
  const [botStatus, setBotStatus] = useState<any>(null)
  const [liveEvents, setLiveEvents] = useState<any[]>([])
  const [logs, setLogs] = useState<any[]>([])
  const [stopResults, setStopResults] = useState<any>(null)

  const handleWsMessage = useCallback((event: any) => {
    if (event.type === 'StatsUpdate') {
      setStats(event.data)
    } else if (event.type === 'BotStatusChanged') {
      setBotStatus(event.data)
      const m = event.data?.mode || 'test'
      api.getTrades(50, 0, m).then(setTrades).catch(() => {})
      api.getStats(m).then(setStats).catch(() => {})
    } else if (event.type === 'BalanceUpdate') {
      setBalance(event.data)
    } else if (event.type === 'TradeDetected' || event.type === 'TradeResolved') {
      setLiveEvents(prev => [event, ...prev].slice(0, 50))
      loadData()
    } else if (event.type === 'LogEntry') {
      setLogs(prev => [...prev, event.data].slice(-2000))
    }
  }, [])

  const { connected } = useWebSocket(handleWsMessage)

  const loadData = async () => {
    try {
      const status = await api.getStatus()
      setBotStatus(status)
      const mode = status?.mode || 'test'
      const [s, t, bal] = await Promise.all([
        api.getStats(mode),
        api.getTrades(50, 0, mode),
        api.getBalance(),
      ])
      setStats(s)
      setTrades(t)
      setBalance(bal)
    } catch (e) {
      console.error('Failed to load data:', e)
    }
  }

  useEffect(() => { loadData() }, [])

  const handleStopBot = async () => {
    try {
      const result = await api.stopBot()
      if (result.results) {
        setStopResults(result.results)
      }
      loadData()
    } catch { loadData() }
  }

  const tabs: { id: Tab; label: string }[] = [
    { id: 'dashboard', label: 'Dashboard' },
    { id: 'analyze', label: 'Analyze' },
    { id: 'history', label: 'History' },
    { id: 'logs', label: 'Logs' },
    { id: 'settings', label: 'Settings' },
  ]

  return (
    <div className="min-h-screen bg-[#0a0a0f]">
      {/* Stop results modal */}
      {stopResults && (
        <div className="fixed inset-0 bg-black/80 flex items-center justify-center z-50 p-4" onClick={() => setStopResults(null)}>
          <div className="bg-zinc-900 border border-zinc-700 rounded-xl max-w-md w-full p-6" onClick={e => e.stopPropagation()}>
            <h3 className="text-lg font-bold text-center mb-4">Session Results</h3>
            <div className="space-y-3">
              <div className="flex justify-between items-center py-2 border-b border-zinc-800">
                <span className="text-zinc-400">Initial Capital</span>
                <span className="font-mono font-bold">${stopResults.initial_capital?.toFixed(2)}</span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-zinc-800">
                <span className="text-zinc-400">Final Balance</span>
                <span className="font-mono font-bold">${stopResults.current_balance?.toFixed(2)}</span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-zinc-800">
                <span className="text-zinc-400">Net P&L</span>
                <span className={`font-mono font-bold text-lg ${stopResults.total_pnl >= 0 ? 'text-emerald-400' : 'text-red-400'}`}>
                  {stopResults.total_pnl >= 0 ? '+' : ''}${stopResults.total_pnl?.toFixed(2)}
                </span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-zinc-800">
                <span className="text-zinc-400">Return</span>
                <span className={`font-mono font-bold text-lg ${stopResults.pnl_pct >= 0 ? 'text-emerald-400' : 'text-red-400'}`}>
                  {stopResults.pnl_pct >= 0 ? '+' : ''}{stopResults.pnl_pct?.toFixed(2)}%
                </span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-zinc-800">
                <span className="text-zinc-400">Trades</span>
                <span className="font-mono">{stopResults.total_trades}</span>
              </div>
              <div className="flex justify-between items-center py-2 border-b border-zinc-800">
                <span className="text-zinc-400">Win / Loss</span>
                <span className="font-mono">{stopResults.wins} / {stopResults.losses}</span>
              </div>
              <div className="flex justify-between items-center py-2">
                <span className="text-zinc-400">Win Rate</span>
                <span className="font-mono">{stopResults.win_rate?.toFixed(1)}%</span>
              </div>
            </div>
            <button onClick={() => setStopResults(null)} className="w-full mt-5 py-2.5 bg-zinc-800 hover:bg-zinc-700 rounded-lg text-sm font-medium">
              Close
            </button>
          </div>
        </div>
      )}

      {/* Header */}
      <header className="border-b border-zinc-800 px-6 py-4">
        <div className="max-w-7xl mx-auto flex items-center justify-between">
          <div className="flex items-center gap-4">
            <h1 className="text-xl font-bold text-white tracking-tight">
              Polymarket CopyBot
            </h1>
            <div className="flex items-center gap-1.5 text-xs">
              <div className={`w-1.5 h-1.5 rounded-full ${connected ? 'bg-emerald-400' : 'bg-red-400'}`} />
              <span className="text-zinc-500">{connected ? 'WS Connected' : 'Disconnected'}</span>
            </div>
          </div>
          <BotControl status={botStatus} onUpdate={loadData} onStop={handleStopBot} />
        </div>
      </header>

      {/* Balance bar */}
      {balance && (
        <div className="border-b border-zinc-800 px-6 py-2 bg-zinc-900/50">
          <div className="max-w-7xl mx-auto flex items-center gap-6 text-sm">
            <div>
              <span className="text-zinc-500">Capital: </span>
              <span className="font-mono font-bold">${balance.initial_capital?.toFixed(2)}</span>
            </div>
            <div>
              <span className="text-zinc-500">Balance: </span>
              <span className="font-mono font-bold">${balance.current_balance?.toFixed(2)}</span>
            </div>
            <div>
              <span className="text-zinc-500">P&L: </span>
              <span className={`font-mono font-bold ${balance.total_pnl >= 0 ? 'text-emerald-400' : 'text-red-400'}`}>
                {balance.total_pnl >= 0 ? '+' : ''}${balance.total_pnl?.toFixed(2)} ({balance.pnl_pct >= 0 ? '+' : ''}{balance.pnl_pct?.toFixed(2)}%)
              </span>
            </div>
            <div>
              <span className="text-zinc-500">Open: </span>
              <span className="font-mono text-amber-400">${balance.open_positions_value?.toFixed(2)}</span>
            </div>
            <div>
              <span className="text-zinc-500">Available: </span>
              <span className="font-mono">${balance.available_capital?.toFixed(2)}</span>
            </div>
          </div>
        </div>
      )}

      {/* Tabs */}
      <nav className="border-b border-zinc-800 px-6">
        <div className="max-w-7xl mx-auto flex gap-0">
          {tabs.map(t => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              className={`px-5 py-3 text-sm font-medium border-b-2 transition-colors ${
                tab === t.id
                  ? 'border-blue-500 text-white'
                  : 'border-transparent text-zinc-500 hover:text-zinc-300'
              }`}
            >
              {t.label}
            </button>
          ))}
        </div>
      </nav>

      {/* Content */}
      <main className="max-w-7xl mx-auto px-6 py-6 space-y-6">
        {tab === 'dashboard' && (
          <>
            <StatsCards stats={stats} />
            <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
              <div className="lg:col-span-2">
                <div className="bg-zinc-900/50 border border-zinc-800 rounded-xl">
                  <div className="px-4 py-3 border-b border-zinc-800">
                    <h2 className="font-semibold text-sm">Recent Trades</h2>
                  </div>
                  <TradesTable trades={trades.slice(0, 15)} />
                </div>
              </div>
              <div>
                <div className="bg-zinc-900/50 border border-zinc-800 rounded-xl">
                  <div className="px-4 py-3 border-b border-zinc-800">
                    <h2 className="font-semibold text-sm">Live Feed</h2>
                  </div>
                  <div className="p-3">
                    <LiveFeed events={liveEvents} />
                  </div>
                </div>
              </div>
            </div>
          </>
        )}

        {tab === 'analyze' && <Analyze />}

        {tab === 'history' && (
          <div className="bg-zinc-900/50 border border-zinc-800 rounded-xl">
            <div className="px-4 py-3 border-b border-zinc-800 flex items-center justify-between">
              <h2 className="font-semibold text-sm">Trade History</h2>
              <div className="flex gap-2">
                <button onClick={() => api.getTrades(100, 0, 'test').then(setTrades)} className="text-xs px-3 py-1 rounded bg-amber-900/50 text-amber-300 hover:bg-amber-900">Test Only</button>
                <button onClick={() => api.getTrades(100, 0, 'live').then(setTrades)} className="text-xs px-3 py-1 rounded bg-emerald-900/50 text-emerald-300 hover:bg-emerald-900">Live Only</button>
                <button onClick={() => api.getTrades(100).then(setTrades)} className="text-xs px-3 py-1 rounded bg-zinc-800 text-zinc-300 hover:bg-zinc-700">All</button>
              </div>
            </div>
            <TradesTable trades={trades} />
          </div>
        )}

        {tab === 'logs' && <LogViewer logs={logs} />}

        {tab === 'settings' && <Settings onAnalyze={(w) => { setTab('analyze'); (window as any).__analyzeWallet = w }} />}
      </main>
    </div>
  )
}
