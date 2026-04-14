const BASE = '/api'

async function request<T>(path: string, opts?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { 'Content-Type': 'application/json' },
    ...opts,
  })
  if (!res.ok) throw new Error(await res.text())
  return res.json()
}

export const api = {
  // Config
  getConfig: () => request<any[]>('/config'),
  setConfig: (key: string, value: any) =>
    request('/config', { method: 'PUT', body: JSON.stringify({ key, value }) }),

  // Wallets
  getWallets: () => request<any[]>('/wallets'),
  addWallet: (address: string, label: string, pnl = 0, volume = 0) =>
    request('/wallets', { method: 'POST', body: JSON.stringify({ address, label, pnl, volume }) }),
  toggleWallet: (id: number, enabled: boolean) =>
    request(`/wallets/${id}`, { method: 'PUT', body: JSON.stringify({ enabled }) }),
  deleteWallet: (id: number) =>
    request(`/wallets/${id}`, { method: 'DELETE' }),

  // Trades
  getTrades: (limit = 50, offset = 0, mode?: string) => {
    const params = new URLSearchParams({ limit: String(limit), offset: String(offset) })
    if (mode) params.set('mode', mode)
    return request<any[]>(`/trades?${params}`)
  },
  getStats: (mode?: string) => {
    const params = mode ? `?mode=${mode}` : ''
    return request<any>(`/trades/stats${params}`)
  },

  // Bot
  getStatus: () => request<any>('/status'),
  getBalance: () => request<any>('/balance'),
  startBot: () => request('/bot/start', { method: 'POST' }),
  stopBot: () => request<any>('/bot/stop', { method: 'POST' }),
  setMode: (mode: string) =>
    request('/bot/mode', { method: 'POST', body: JSON.stringify({ mode }) }),

  // Leaderboard
  getLeaderboard: (category = 'CRYPTO', period = 'WEEK', limit = 20) =>
    request<any[]>(`/leaderboard?category=${category}&period=${period}&limit=${limit}`),

  // Analyze
  analyzeWallet: (wallet: string) => request<any>(`/analyze/${wallet}`),

  // BTC markets
  getBtcMarkets: () => request<any[]>('/markets/btc'),
}
