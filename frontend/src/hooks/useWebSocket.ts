import { useEffect, useRef, useState } from 'react'

type WsEvent = {
  type: string
  data: any
}

export function useWebSocket(onMessage: (event: WsEvent) => void) {
  const wsRef = useRef<WebSocket | null>(null)
  const onMessageRef = useRef(onMessage)
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [connected, setConnected] = useState(false)

  // Keep callback ref current without triggering reconnects
  onMessageRef.current = onMessage

  useEffect(() => {
    let disposed = false

    function connect() {
      if (disposed) return

      const proto = window.location.protocol === 'https:' ? 'wss' : 'ws'
      const ws = new WebSocket(`${proto}://${window.location.host}/ws`)

      ws.onopen = () => {
        if (!disposed) setConnected(true)
      }

      ws.onclose = () => {
        if (!disposed) {
          setConnected(false)
          reconnectTimer.current = setTimeout(connect, 3000)
        }
      }

      ws.onerror = () => {
        ws.close()
      }

      ws.onmessage = (e) => {
        try {
          const event = JSON.parse(e.data) as WsEvent
          onMessageRef.current(event)
        } catch { /* ignore bad messages */ }
      }

      wsRef.current = ws
    }

    connect()

    return () => {
      disposed = true
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current)
      wsRef.current?.close()
    }
  }, []) // empty deps — connect once

  return { connected }
}
