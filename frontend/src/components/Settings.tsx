import { useState, useEffect } from 'react'
import { api } from '../hooks/useApi'
import { WalletManager } from './WalletManager'

const STRATEGIES = [
  { value: 'oracle', label: 'Oracle Lag', desc: 'Explora atraso de ~55s entre preço real e odds — entra no último minuto (mais seguro)' },
  { value: 'hedge', label: 'Hedge Arbitrage', desc: 'Compra Up+Down quando soma < threshold — lucro garantido' },
  { value: 'mm', label: 'Market Making (Stoikov)', desc: 'Quotes dinâmicos bid/ask em mercados 5min — lucra com spread e mispricing' },
  { value: 'copy', label: 'Copy Trading', desc: 'Copia trades das wallets monitoradas' },
]

const CONFIG_FIELDS = [
  { key: 'simulated_capital', label: 'Simulated Capital (USDC)', type: 'number' },
  { key: 'max_per_trade', label: 'Max Per Trade (USDC)', type: 'number' },
  { key: 'max_open_positions', label: 'Max Open Positions', type: 'number' },
  { key: 'max_consecutive_losses', label: 'Max Consecutive Losses (circuit breaker)', type: 'number' },
  { key: 'daily_loss_limit', label: 'Daily Loss Limit (USDC)', type: 'number' },
  { key: 'slippage_estimate_pct', label: 'Slippage Estimate (%)', type: 'number' },
  { key: 'max_price_drift_pct', label: 'Max Price Drift (%)', type: 'number' },
  { key: 'min_trade_size', label: 'Min Trade Size (USDC)', type: 'number' },
  { key: 'oracle_entry_window_secs', label: 'Oracle Entry Window (seconds before close)', type: 'number' },
  { key: 'oracle_min_move_pct', label: 'Oracle Min Move % (0.07)', type: 'number' },
  { key: 'oracle_max_token_price', label: 'Oracle Max Token Price (0.62)', type: 'number' },
  { key: 'hedge_threshold', label: 'Hedge Threshold (soma max)', type: 'number' },
  { key: 'hedge_amount_per_leg', label: 'Hedge $ per Leg', type: 'number' },
  { key: 'max_hedges_open', label: 'Max Hedges Open', type: 'number' },
  { key: 'mm_gamma', label: 'MM Gamma (risk aversion, 0.35)', type: 'number' },
  { key: 'mm_sigma', label: 'MM Sigma (volatility, 0.08)', type: 'number' },
  { key: 'mm_k', label: 'MM K (order arrival, 1.8)', type: 'number' },
  { key: 'mm_min_edge', label: 'MM Min Edge (0.01)', type: 'number' },
  { key: 'mm_sensitivity', label: 'MM Sensitivity (sigmoid, 50)', type: 'number' },
]

export function Settings({ onAnalyze }: { onAnalyze?: (wallet: string) => void }) {
  const [config, setConfig] = useState<Record<string, any>>({})
  const [saved, setSaved] = useState(false)
  const [strategy, setStrategy] = useState('both')

  useEffect(() => {
    api.getConfig().then(entries => {
      const cfg: Record<string, any> = {}
      for (const e of entries) cfg[e.key] = e.value
      setConfig(cfg)
      if (cfg.strategy) setStrategy(String(cfg.strategy))
    })
  }, [])

  const handleSave = async (key: string, value: string) => {
    const num = parseFloat(value)
    if (isNaN(num)) return
    await api.setConfig(key, num)
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  const handleStrategy = async (val: string) => {
    setStrategy(val)
    await api.setConfig('strategy', val)
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  return (
    <div className="space-y-8">
      {/* Strategy selector */}
      <div>
        <h2 className="text-lg font-semibold mb-4">Strategy</h2>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
          {STRATEGIES.map(s => (
            <button
              key={s.value}
              onClick={() => handleStrategy(s.value)}
              className={`p-4 rounded-xl border text-left transition-colors ${
                strategy === s.value
                  ? 'border-blue-500 bg-blue-500/10'
                  : 'border-zinc-800 bg-zinc-900 hover:border-zinc-700'
              }`}
            >
              <div className="font-semibold text-sm">{s.label}</div>
              <div className="text-xs text-zinc-400 mt-1">{s.desc}</div>
            </button>
          ))}
        </div>
        <div className="text-xs text-zinc-500 mt-2">Reinicie o bot após mudar a estratégia</div>
      </div>

      {/* Config */}
      <div>
        <h2 className="text-lg font-semibold mb-4">Configuration</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
          {CONFIG_FIELDS.map(f => (
            <div key={f.key} className="bg-zinc-900 border border-zinc-800 rounded-lg p-4">
              <label className="text-xs text-zinc-500 uppercase tracking-wider block mb-2">{f.label}</label>
              <input
                type="number"
                step="any"
                value={config[f.key] ?? ''}
                onChange={e => setConfig(prev => ({ ...prev, [f.key]: e.target.value }))}
                onBlur={e => handleSave(f.key, e.target.value)}
                className="w-full bg-zinc-800 border border-zinc-700 rounded px-3 py-2 text-sm font-mono focus:border-blue-500 outline-none"
              />
            </div>
          ))}
        </div>
        {saved && <div className="text-emerald-400 text-sm mt-2">Saved</div>}
      </div>

      {/* Wallets (only shown for copy/both) */}
      {strategy !== 'snipe' && <WalletManager onAnalyze={onAnalyze} />}
    </div>
  )
}
