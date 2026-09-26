import { useEffect, useMemo, useRef, useState } from 'react'
import { sampleReply, sampleText } from './mocks/demo'
import type { Mapping } from './api'
import {
  decodeText,
  deleteMapping,
  encodeText,
  extractFile,
  fileToBase64,
  health,
  upsertMapping,
  API_STAGE,
} from './api'
import { extractPdfText } from './pdfExtract'
import type { DocTab } from './types'
import { DocumentTabs } from './components/DocumentTabs'
import { Vault } from './components/Vault'
import './App.css'

type Page = 'Encode' | 'Decode' | 'Vault'

type Document = {
  id: number
  name: string
  input: string
  reply: string
  output: string
  restored: string
  unknown: string[]
  mappings: Mapping[]
  nextIdx: Record<string, number>
}

function newDocument(id: number, name?: string): Document {
  return {
    id,
    name: name ?? `document-${id}.txt`,
    input: '',
    reply: '',
    output: '',
    restored: '',
    unknown: [],
    mappings: [],
    nextIdx: {},
  }
}

function loadSessionId(): string | null {
  try {
    return window.localStorage.getItem('poco.sessionId')
  } catch {
    return null
  }
}

function saveSessionId(id: string) {
  try {
    window.localStorage.setItem('poco.sessionId', id)
  } catch {
    /* ignore */
  }
}

function NavIcon({ page }: { page: Page }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true">
      {page === 'Vault' ? (
        <>
          <rect x="3" y="4" width="18" height="5" rx="1" />
          <path d="M5 9v11h14V9M10 13h4" />
        </>
      ) : (
        <>
          <rect x="5" y="10" width="14" height="11" rx="2" />
          <path d={page === 'Encode' ? 'M8 10V6a4 4 0 0 1 8 0v4' : 'M8 10V6a4 4 0 0 1 8 0'} />
        </>
      )}
    </svg>
  )
}

function TokenText({ text, mappings }: { text: string; mappings: Mapping[] }) {
  return text.split(/(\[[A-Za-z][A-Za-z0-9_:]*_\d+\])/g).map((part, index) => {
    const mapping = mappings.find((m) => `[${m.token}]` === part)
    return mapping ? (
      <mark key={index} className={`token-${mapping.category.toLowerCase()}`}>
        {part}
      </mark>
    ) : (
      part
    )
  })
}

function wordAtSelection(textarea: HTMLTextAreaElement): string {
  const text = textarea.value
  let start = textarea.selectionStart
  let end = textarea.selectionEnd
  if (start === end) {
    while (start > 0 && /[\p{L}\p{N}'’@._+-]/u.test(text[start - 1] ?? '')) start -= 1
    while (end < text.length && /[\p{L}\p{N}'’@._+-]/u.test(text[end] ?? '')) end += 1
  }
  return text.slice(start, end).trim()
}

const CLASS_OPTIONS = ['Name', 'Email', 'Phone', 'Location', 'Date', 'Passport', 'Ssn', 'Iban', 'CreditCard', 'Organization', 'Custom']

function guessClass(word: string): string {
  if (/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(word)) return 'Email'
  if (/^[+()\d][\d\s()-]{6,}$/.test(word)) return 'Phone'
  if (/^\d{4}[-\s]?\d{4}/.test(word.replace(/\s/g, ''))) return 'CreditCard'
  if (/^[A-Z][a-z]+(?:\s+[A-Z][a-z]+)+$/.test(word)) return 'Name'
  return 'Custom'
}

function App() {
  const [page, setPage] = useState<Page>('Encode')
  const [light, setLight] = useState(false)
  /** 1. One shared documents array — single source of truth. */
  const [documents, setDocuments] = useState<Document[]>(() => [newDocument(1)])
  /** 2. Two independent active-id pointers, one per view. */
  const [activeEncode, setActiveEncode] = useState(1)
  const [activeVault, setActiveVault] = useState(1)
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  const [apiStatus, setApiStatus] = useState<string>('checking…')
  const [sessionId, setSessionId] = useState<string | null>(loadSessionId)
  const [draftToken, setDraftToken] = useState<{ value: string; cls: string } | null>(null)
  const fileInput = useRef<HTMLInputElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  /** 3. Naming counter only — never used as an id. */
  const docCounter = useRef(1)

  const encoding = page === 'Encode'
  const activeViewId = page === 'Vault' ? activeVault : activeEncode
  const active = documents.find((doc) => doc.id === activeViewId) ?? documents[0]
  const input = encoding ? active.input : active.reply
  const output = encoding ? active.output : active.restored

  useEffect(() => {
    health()
      .then((h) => setApiStatus(`API ${h.env} · v${h.version}`))
      .catch(() => setApiStatus('API offline'))
  }, [])

  const tabDocs: DocTab[] = useMemo(() => documents.map((d) => ({ id: d.id, name: d.name })), [documents])

  function updateDocument(id: number, changes: Partial<Document>) {
    setDocuments((previous) => previous.map((doc) => (doc.id === id ? { ...doc, ...changes } : doc)))
  }

  function navigate(next: Page) {
    setPage(next)
    setNotice('')
    setDraftToken(null)
  }

  /** Jump keeps the same document scoped in the destination view. */
  function jumpTo(target: Page) {
    if (target === 'Vault') setActiveVault(activeEncode)
    else setActiveEncode(activeVault)
    navigate(target)
  }

  /** 4. Close reconciles BOTH views' active ids. × is only rendered when length > 1. */
  function closeDocument(id: number) {
    if (documents.length <= 1) return
    const remaining = documents.filter((doc) => doc.id !== id)
    setDocuments(remaining)
    if (activeEncode === id) setActiveEncode(remaining[0].id)
    if (activeVault === id) setActiveVault(remaining[0].id)
    setNotice('Document closed.')
    setDraftToken(null)
  }

  /** Add: new empty doc; both views jump to it. */
  function addDocument() {
    docCounter.current += 1
    const id = Date.now() + docCounter.current
    const doc = newDocument(id, `document-${docCounter.current}.txt`)
    setDocuments((previous) => [...previous, doc])
    setActiveEncode(doc.id)
    setActiveVault(doc.id)
    setNotice('')
    setDraftToken(null)
  }

  async function encode() {
    setBusy(true)
    setNotice('')
    const documentId = active.id
    try {
      const result = await encodeText(sessionId, active.input)
      saveSessionId(result.sessionId)
      setSessionId(result.sessionId)
      const nextIdx: Record<string, number> = {}
      for (const m of result.mappings) {
        const cls = m.category || 'Custom'
        const n = Number((m.token.match(/_(\d+)$/) ?? [])[1] ?? '1')
        nextIdx[cls] = Math.max(nextIdx[cls] ?? 1, n + 1)
      }
      updateDocument(documentId, {
        output: result.redactedText,
        mappings: result.mappings,
        restored: '',
        unknown: [],
        nextIdx,
      })
      setNotice(`${result.mappings.length} values replaced · session ${result.sessionId.slice(0, 8)}…`)
    } catch (e) {
      setNotice(`Encode failed: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setBusy(false)
    }
  }

  async function decode() {
    if (!sessionId) {
      setNotice('No session yet. Encode text first so the server can store mappings.')
      return
    }
    setBusy(true)
    setNotice('')
    try {
      const result = await decodeText(sessionId, active.reply)
      updateDocument(active.id, {
        restored: result.restoredText,
        unknown: result.hallucinations,
      })
      setNotice(
        result.hallucinations.length
          ? `Restored with ${result.hallucinations.length} unknown token(s) left unchanged.`
          : 'Response restored from the server session vault.'
      )
    } catch (e) {
      setNotice(`Decode failed: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setBusy(false)
    }
  }

  async function changeMappings(mappings: Mapping[]) {
    const previous = active.mappings
    const documentId = active.id
    if (sessionId) {
      try {
        for (const old of previous) {
          if (!mappings.some((m) => m.token === old.token)) {
            await deleteMapping(sessionId, old.token)
          }
        }
        for (const next of mappings) {
          const before = previous.find((p) => p.token === next.token)
          if (!before || before.value !== next.value || before.category !== next.category) {
            await upsertMapping(sessionId, {
              token: next.token,
              value: next.value,
              category: next.category,
            })
          }
        }
      } catch (e) {
        setNotice(`Vault sync failed: ${e instanceof Error ? e.message : String(e)}`)
        return
      }
    }
    updateDocument(documentId, { mappings, restored: '', unknown: [] })
    setNotice(sessionId ? 'Vault updated on the server.' : 'Local vault updated. Encode first to persist.')
  }

  function addTokenFromWord(word: string) {
    const cls = guessClass(word)
    setDraftToken({ value: word, cls })
    setActiveVault(active.id)
    setPage('Vault')
    setNotice(`New token drafted from “${word}”. Choose a class and save.`)
  }

  function commitDraftToken() {
    if (!draftToken) return
    const cls = draftToken.cls
    const n = active.nextIdx[cls] ?? 1
    const token = `${cls}_${n}`
    const mapping: Mapping = {
      token,
      value: draftToken.value,
      category: cls,
      firstOffset: active.input.indexOf(draftToken.value),
    }
    updateDocument(active.id, { nextIdx: { ...active.nextIdx, [cls]: n + 1 } })
    void changeMappings([...active.mappings, mapping])
    setDraftToken(null)
  }

  /**
   * 5. Upload writes only to the active document.
   * PDF text is extracted in the browser (avoids HTTP 413 on large PDFs).
   * DOCX still goes through /extract (base64) — those files are much smaller.
   */
  async function upload(file?: File) {
    if (!file) return
    if (!/\.(txt|md|pdf|docx)$/i.test(file.name)) {
      setNotice('Choose a .txt, .md, .pdf, or .docx file.')
      return
    }
    if (file.size > 25_000_000) {
      setNotice('Choose a file smaller than 25 MB.')
      return
    }
    const documentId = active.id
    setBusy(true)
    try {
      let text = ''
      if (/\.pdf$/i.test(file.name)) {
        text = await extractPdfText(file)
        setNotice(`Extracted ${text.length} characters from ${file.name} in the browser.`)
      } else if (/\.docx$/i.test(file.name)) {
        const fileBase64 = await fileToBase64(file)
        const extracted = await extractFile(file.name, { fileBase64 })
        text = extracted.text
        setNotice(`Extracted ${text.length} characters from ${file.name}.`)
      } else {
        text = await file.text()
        setNotice(`File loaded. Ready to ${encoding ? 'encode' : 'decode'}.`)
      }
      updateDocument(
        documentId,
        encoding ? { name: file.name, input: text, output: '' } : { reply: text, restored: '', unknown: [] }
      )
    } catch (e) {
      setNotice(`Upload failed: ${e instanceof Error ? e.message : String(e)}`)
    } finally {
      setBusy(false)
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      setNotice('Copied to clipboard.')
    } catch {
      setNotice('Could not copy automatically. Select the output and copy it manually.')
    }
  }

  function handleInputClick() {
    const el = inputRef.current
    if (!el || !encoding) return
    const word = wordAtSelection(el)
    if (word && word.length >= 2) addTokenFromWord(word)
  }

  const descriptions = {
    Encode: 'Replace sensitive information with tokens. Click a word in the input to draft a token.',
    Decode: 'Restore tokens using the server session vault.',
    Vault: 'Inspect and edit mappings for the active document.',
  }

  return (
    <div className={`app-shell${light ? ' light' : ''}`}>
      <aside className="sidebar">
        <button className="brand" onClick={() => navigate('Encode')} aria-label="Poco home">
          <span className="logo-frame">
            <img src={`${import.meta.env.BASE_URL}poco_logo.png`} alt="" />
          </span>
          <span>Poco</span>
        </button>
        <nav aria-label="Main navigation">
          {(['Encode', 'Decode', 'Vault'] as const).map((item) => (
            <button
              key={item}
              className={`nav-item${page === item ? ' active' : ''}`}
              aria-current={page === item ? 'page' : undefined}
              onClick={() => jumpTo(item)}
            >
              <NavIcon page={item} />
              <span>
                <strong>{item}</strong>
                <small>
                  {
                    {
                      Encode: 'Protect sensitive text',
                      Decode: 'Restore original text',
                      Vault: 'Inspect stored mappings',
                    }[item]
                  }
                </small>
              </span>
            </button>
          ))}
        </nav>
        <p className="demo-note">
          Stage: <strong>{API_STAGE}</strong>
          <br />
          {apiStatus}
          {sessionId ? (
            <>
              <br />
              Session: <code>{sessionId.slice(0, 8)}…</code>
            </>
          ) : null}
        </p>
      </aside>
      <div className="main-shell">
        <main>
          <div className="page-heading">
            <div>
              <h1>{page}</h1>
              <p>{descriptions[page]}</p>
            </div>
            <div className="page-actions">
              {page === 'Vault' && (
                <button
                  className="danger"
                  disabled={!active.mappings.length}
                  onClick={() => {
                    void changeMappings([])
                  }}
                >
                  Reset vault
                </button>
              )}
              <button
                className="theme-button"
                onClick={() => setLight(!light)}
                aria-label={`Switch to ${light ? 'dark' : 'light'} theme`}
              >
                <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
                  <circle cx="12" cy="12" r="4" />
                  <path d="M12 1v2m0 18v2M1 12h2m18 0h2M4 4l2 2m12 12 2 2M4 20l2-2M18 6l2-2" />
                </svg>
              </button>
            </div>
          </div>

          {/* Shared strip: each view passes its own activeId + switch callback */}
          <DocumentTabs
            documents={tabDocs}
            activeId={active.id}
            onSelect={(id) => {
              if (page === 'Vault') setActiveVault(id)
              else setActiveEncode(id)
              setNotice('')
              setDraftToken(null)
            }}
            onClose={closeDocument}
            onAdd={addDocument}
          />

          {page !== 'Vault' && (
            <>
              <div className="editors">
                <section className="editor">
                  <div className="editor-heading">
                    <label htmlFor="source-input">Input</label>
                    <div>
                      <button className="quiet" onClick={() => fileInput.current?.click()}>
                        Upload file
                      </button>
                      <button
                        className="quiet"
                        onClick={() => {
                          updateDocument(
                            active.id,
                            encoding
                              ? { input: sampleText, name: 'onboarding-dossier.txt', output: '' }
                              : { reply: sampleReply, restored: '', unknown: [] }
                          )
                          setNotice('Example loaded.')
                        }}
                      >
                        Use example
                      </button>
                      <input
                        ref={fileInput}
                        type="file"
                        accept=".txt,.md,.pdf,.docx,text/plain,text/markdown,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                        className="sr-only"
                        tabIndex={-1}
                        aria-label="Upload text, PDF, or DOCX file"
                        onChange={(e) => {
                          void upload(e.target.files?.[0])
                          e.target.value = ''
                        }}
                      />
                    </div>
                  </div>
                  <textarea
                    id="source-input"
                    ref={inputRef}
                    value={input}
                    onChange={(e) => {
                      updateDocument(
                        active.id,
                        encoding ? { input: e.target.value, output: '' } : { reply: e.target.value, restored: '', unknown: [] }
                      )
                      setNotice('')
                    }}
                    onClick={handleInputClick}
                    placeholder={
                      encoding
                        ? 'Paste text here or click a word to draft a token…'
                        : 'Paste text with tokens here or upload a file…'
                    }
                    spellCheck={false}
                  />
                </section>
                <section className="editor">
                  <div className="editor-heading">
                    <h2 id="output-label">Output</h2>
                    <button className="quiet" disabled={!output} onClick={() => copy(output)}>
                      Copy
                    </button>
                  </div>
                  <pre className="output" tabIndex={0} aria-labelledby="output-label">
                    {output ? (
                      encoding ? (
                        <TokenText text={output} mappings={active.mappings} />
                      ) : (
                        output
                      )
                    ) : (
                      <span className="placeholder">
                        {encoding ? 'Run Encode to see tokenized output.' : 'Run Decode to see restored text.'}
                      </span>
                    )}
                  </pre>
                </section>
              </div>
              <div className="actions">
                <button
                  className="primary"
                  disabled={!input.trim() || busy}
                  onClick={() => void (encoding ? encode() : decode())}
                >
                  {busy ? 'Working…' : page}
                </button>
                <button className="secondary" onClick={() => jumpTo('Vault')}>
                  Inspect PII
                </button>
              </div>
              {!encoding && !!active.unknown.length && (
                <p className="warning" role="alert">
                  Unknown tokens left unchanged: {active.unknown.join(', ')}
                </p>
              )}
            </>
          )}

          {page === 'Vault' && (
            <>
              {draftToken && (
                <div className="draft-token" role="region" aria-label="New token">
                  <strong>New token</strong>
                  <input
                    aria-label="Token value"
                    value={draftToken.value}
                    onChange={(e) => setDraftToken({ ...draftToken, value: e.target.value })}
                  />
                  <select
                    aria-label="Token class"
                    value={draftToken.cls}
                    onChange={(e) => setDraftToken({ ...draftToken, cls: e.target.value })}
                  >
                    {CLASS_OPTIONS.map((c) => (
                      <option key={c}>{c}</option>
                    ))}
                  </select>
                  <button className="primary" onClick={commitDraftToken}>
                    Save
                  </button>
                  <button className="quiet" onClick={() => setDraftToken(null)}>
                    Cancel
                  </button>
                </div>
              )}
              <Vault
                key={active.id}
                mappings={active.mappings}
                onChange={(next) => {
                  void changeMappings(next)
                }}
                onAdd={() => {
                  const cls = 'Custom'
                  const n = active.nextIdx[cls] ?? 1
                  const mapping: Mapping = {
                    token: `${cls}_${n}`,
                    value: 'new value',
                    category: cls,
                    firstOffset: 0,
                  }
                  updateDocument(active.id, { nextIdx: { ...active.nextIdx, [cls]: n + 1 } })
                  void changeMappings([...active.mappings, mapping])
                }}
              />
            </>
          )}
          <p className="status" role="status">
            {notice}
          </p>
        </main>
      </div>
    </div>
  )
}

export default App
