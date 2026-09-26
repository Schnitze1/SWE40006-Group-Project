export type Mapping = {
  token: string
  value: string
  category: string
  firstOffset: number
}

export type EncodeResult = {
  sessionId: string
  redactedText: string
  mappings: Mapping[]
  stats: { totalEntities: number; durationMs: number }
}

export type DecodeResult = {
  sessionId: string
  restoredText: string
  hallucinations: string[]
}

const STAGE = (import.meta.env.VITE_API_STAGE as string | undefined) || 'dev'
const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ||
  `https://6xz841x652.execute-api.us-east-1.amazonaws.com/${STAGE}`

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  const text = await res.text()
  let data: unknown = null
  try {
    data = text ? JSON.parse(text) : null
  } catch {
    throw new Error(text || `HTTP ${res.status}`)
  }
  if (!res.ok) {
    const msg =
      data && typeof data === 'object' && 'error' in data
        ? String((data as { error: unknown }).error)
        : `HTTP ${res.status}`
    throw new Error(msg)
  }
  return data as T
}

export async function encodeText(sessionId: string | null, text: string): Promise<EncodeResult> {
  return post<EncodeResult>('/encode', sessionId ? { sessionId, text } : { text })
}

export async function decodeText(sessionId: string, text: string): Promise<DecodeResult> {
  return post<DecodeResult>('/decode', { sessionId, text })
}

export async function health(): Promise<{ status: string; version: string; env: string }> {
  const res = await fetch(`${BASE}/health`)
  if (!res.ok) throw new Error(`health HTTP ${res.status}`)
  return res.json()
}

export { BASE as API_BASE, STAGE as API_STAGE }
