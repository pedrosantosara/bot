import { api } from '../hooks/useApi'

type Props = {
  status: any
  onUpdate: () => void
  onStop?: () => void
}

export function BotControl({ status, onUpdate, onStop }: Props) {
  const running = status?.running
  const mode = status?.mode || 'test'

  const handleToggle = async () => {
    if (running) {
      if (onStop) onStop()
      else { await api.stopBot(); onUpdate() }
    } else {
      await api.startBot()
      onUpdate()
    }
  }

  const handleMode = async (m: string) => {
    if (m === 'live' && !confirm('⚠️ Modo LIVE usa dinheiro REAL na Polymarket. Continuar?')) return
    await api.setMode(m)
    onUpdate()
  }

  return (
    <div className="flex items-center gap-2 sm:gap-4 flex-wrap">
      <div className="flex bg-zinc-900 rounded-lg border border-zinc-800 p-0.5">
        <button
          onClick={() => handleMode('test')}
          className={`px-3 sm:px-4 py-1.5 rounded-md text-xs sm:text-sm font-medium transition-colors ${
            mode === 'test' ? 'bg-amber-600 text-white' : 'text-zinc-400 hover:text-zinc-200'
          }`}
        >
          Test
        </button>
        <button
          onClick={() => handleMode('live')}
          className={`px-3 sm:px-4 py-1.5 rounded-md text-xs sm:text-sm font-medium transition-colors ${
            mode === 'live' ? 'bg-emerald-600 text-white' : 'text-zinc-400 hover:text-zinc-200'
          }`}
        >
          Live
        </button>
      </div>

      <button
        onClick={handleToggle}
        className={`px-4 sm:px-6 py-2 rounded-lg font-medium text-xs sm:text-sm transition-colors ${
          running
            ? 'bg-red-600 hover:bg-red-700 text-white'
            : 'bg-emerald-600 hover:bg-emerald-700 text-white'
        }`}
      >
        {running ? '■ Stop' : '▶ Start'}
      </button>

      <div className="flex items-center gap-1.5 text-xs sm:text-sm">
        <div className={`w-2 h-2 rounded-full ${running ? 'bg-emerald-400 animate-pulse' : 'bg-zinc-600'}`} />
        <span className="text-zinc-400">
          {running ? mode : 'Off'}
        </span>
      </div>
    </div>
  )
}
