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

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
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

async function post<T>(path: string, body: unknown): Promise<T> {
  return request<T>('POST', path, body)
}

export async function encodeText(
  sessionId: string | null,
  text: string,
  file?: { fileName: string; fileBase64?: string; s3Key?: string }
): Promise<EncodeResult> {
  const body: Record<string, unknown> = {}
  if (sessionId) body.sessionId = sessionId
  if (file?.s3Key) {
    body.fileName = file.fileName
    body.s3Key = file.s3Key
  } else if (file?.fileBase64) {
    body.fileName = file.fileName
    body.fileBase64 = file.fileBase64
  } else {
    body.text = text
  }
  return post<EncodeResult>('/encode', body)
}

export async function extractFile(
  fileName: string,
  opts: { fileBase64?: string; s3Key?: string }
): Promise<{ fileName: string; text: string }> {
  if (opts.s3Key) return post('/extract', { fileName, s3Key: opts.s3Key })
  return post('/extract', { fileName, fileBase64: opts.fileBase64 })
}

export function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = reader.result
      if (typeof result !== 'string') {
        reject(new Error('Could not read file'))
        return
      }
      const comma = result.indexOf(',')
      resolve(comma >= 0 ? result.slice(comma + 1) : result)
    }
    reader.onerror = () => reject(reader.error ?? new Error('Could not read file'))
    reader.readAsDataURL(file)
  })
}

/** Ask Lambda for a presigned PUT URL (bypasses API 6MB payload cap). */
export async function presignUpload(
  fileName: string,
  contentType: string,
  sessionId?: string | null
): Promise<{ sessionId: string; key: string; uploadUrl: string }> {
  const body: Record<string, unknown> = { fileName, contentType }
  if (sessionId) body.sessionId = sessionId
  return post('/presign', body)
}

/** Browser → S3 PUT (no API Gateway size limit). */
export async function uploadToS3(uploadUrl: string, file: Blob, contentType: string): Promise<void> {
  const res = await fetch(uploadUrl, {
    method: 'PUT',
    headers: { 'Content-Type': contentType },
    body: file,
  })
  if (!res.ok) {
    const text = await res.text()
    throw new Error(text || `S3 upload HTTP ${res.status}`)
  }
}

export async function decodeText(sessionId: string, text: string): Promise<DecodeResult> {
  return post<DecodeResult>('/decode', { sessionId, text })
}

export async function health(): Promise<{ status: string; version: string; env: string }> {
  const res = await fetch(`${BASE}/health`)
  if (!res.ok) throw new Error(`health HTTP ${res.status}`)
  return res.json()
}

export async function upsertMapping(
  sessionId: string,
  mapping: { token: string; value: string; category?: string }
): Promise<{ sessionId: string; mapping: Mapping }> {
  return request('PUT', `/sessions/${sessionId}/mappings`, mapping)
}

export async function deleteMapping(sessionId: string, token: string): Promise<void> {
  const res = await fetch(`${BASE}/sessions/${sessionId}/mappings/${encodeURIComponent(token)}`, {
    method: 'DELETE',
  })
  if (!res.ok && res.status !== 204) {
    const text = await res.text()
    throw new Error(text || `HTTP ${res.status}`)
  }
}

export { BASE as API_BASE, STAGE as API_STAGE }
