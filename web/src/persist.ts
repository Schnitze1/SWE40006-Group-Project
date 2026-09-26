/** Persist workspace so a page refresh does not wipe documents or the session. */

const DOCS_KEY = 'poco.docs'
const SESSION_KEY = 'poco.sessionId'
const UI_KEY = 'poco.ui'

export type PersistedDoc = {
  id: number
  name: string
  input: string
  reply: string
  output: string
  restored: string
  unknown: string[]
  mappings: { token: string; value: string; category: string; firstOffset: number }[]
  nextIdx: Record<string, number>
}

export type PersistedState = {
  documents: PersistedDoc[]
  activeEncode: number
  activeVault: number
  sessionId: string | null
  docCounter: number
}

export function loadWorkspace(): PersistedState | null {
  try {
    const raw = window.localStorage.getItem(DOCS_KEY)
    const uiRaw = window.localStorage.getItem(UI_KEY)
    const sessionId = window.localStorage.getItem(SESSION_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as { documents?: PersistedDoc[]; docCounter?: number }
    const ui = uiRaw ? (JSON.parse(uiRaw) as { activeEncode?: number; activeVault?: number }) : {}
    const documents = parsed.documents
    if (!Array.isArray(documents) || documents.length === 0) return null
    return {
      documents,
      activeEncode: ui.activeEncode ?? documents[0].id,
      activeVault: ui.activeVault ?? documents[0].id,
      sessionId,
      docCounter: parsed.docCounter ?? documents.length,
    }
  } catch {
    return null
  }
}

export function saveWorkspace(state: PersistedState) {
  try {
    window.localStorage.setItem(
      DOCS_KEY,
      JSON.stringify({ documents: state.documents, docCounter: state.docCounter })
    )
    window.localStorage.setItem(
      UI_KEY,
      JSON.stringify({ activeEncode: state.activeEncode, activeVault: state.activeVault })
    )
    if (state.sessionId) window.localStorage.setItem(SESSION_KEY, state.sessionId)
  } catch {
    /* quota / private mode */
  }
}

export function loadSession(): string | null {
  try {
    return window.localStorage.getItem(SESSION_KEY)
  } catch {
    return null
  }
}

export function saveSession(id: string) {
  try {
    window.localStorage.setItem(SESSION_KEY, id)
  } catch {
    /* ignore */
  }
}
