import { useRef, useState } from 'react'
import { decodeDemo, encodeDemo, sampleReply, sampleText } from './mocks/demo'
import type { Mapping } from './mocks/demo'
import { Vault } from './components/Vault'
import './App.css'

type Page = 'Encode' | 'Decode' | 'Vault'
type Document = {
  id: number
  name: string
  input: string
  output: string
  reply: string
  restored: string
  unknown: string[]
  mappings: Mapping[]
  nextToken: number
}

function newDocument(id: number): Document {
  return { id, name: `document-${id}.txt`, input: '', output: '', reply: '', restored: '', unknown: [], mappings: [], nextToken: 1 }
}

function NavIcon({ page }: { page: Page }) {
  return <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true">
    {page === 'Vault' ? <><rect x="3" y="4" width="18" height="5" rx="1" /><path d="M5 9v11h14V9M10 13h4" /></> : <><rect x="5" y="10" width="14" height="11" rx="2" /><path d={page === 'Encode' ? 'M8 10V6a4 4 0 0 1 8 0v4' : 'M8 10V6a4 4 0 0 1 8 0'} /></>}
  </svg>
}

function TokenText({ text, mappings }: { text: string; mappings: Mapping[] }) {
  return text.split(/(\[[A-Za-z][A-Za-z0-9_:]*_\d+\])/g).map((part, index) => {
    const mapping = mappings.find((m) => `[${m.token}]` === part)
    return mapping ? <mark key={index} className={`token-${mapping.category.toLowerCase()}`}>{part}</mark> : part
  })
}

function App() {
  const [page, setPage] = useState<Page>('Encode')
  const [light, setLight] = useState(false)
  const [documents, setDocuments] = useState<Document[]>([newDocument(1)])
  const [activeId, setActiveId] = useState(1)
  const [notice, setNotice] = useState('')
  const fileInput = useRef<HTMLInputElement>(null)
  const nextDocumentId = useRef(2)
  const active = documents.find((doc) => doc.id === activeId)!
  const encoding = page === 'Encode'
  const input = encoding ? active.input : active.reply
  const output = encoding ? active.output : active.restored

  function updateDocument(id: number, changes: Partial<Document>) {
    setDocuments((previous) => previous.map((doc) => doc.id === id ? { ...doc, ...changes } : doc))
  }

  function navigate(next: Page) {
    setPage(next)
    setNotice('')
  }

  function addDocument() {
    const doc = newDocument(nextDocumentId.current++)
    setDocuments((previous) => [...previous, doc])
    setActiveId(doc.id)
    setNotice('')
  }

  function encode() {
    const result = encodeDemo(active.input)
    updateDocument(activeId, { output: result.redactedText, mappings: result.mappings, restored: '', unknown: [] })
    setNotice(`${result.mappings.length} sample values replaced. Encoding replaces this document’s vault.`)
  }

  function decode() {
    const result = decodeDemo(active.reply, active.mappings)
    updateDocument(activeId, { restored: result.restoredText, unknown: result.hallucinations })
    setNotice('Response restored using this document’s vault.')
  }

  function changeMappings(mappings: Mapping[]) {
    updateDocument(activeId, { mappings, restored: '', unknown: [] })
    setNotice('Vault updated. Decode again to use the current values.')
  }

  async function upload(file?: File) {
    if (!file) return
    if (!/\.(txt|md)$/i.test(file.name)) { setNotice('Choose a .txt or .md file. PDF and DOCX need the backend.'); return }
    if (file.size > 1_000_000) { setNotice('Choose a text file smaller than 1 MB.'); return }
    const documentId = activeId
    try {
      const input = await file.text()
      updateDocument(documentId, encoding
        ? { name: file.name, input, output: '' }
        : { reply: input, restored: '', unknown: [] })
      setNotice(`File loaded locally. Ready to ${encoding ? 'encode' : 'decode'}.`)
    } catch { setNotice('Could not read this file. Please try again.') }
  }

  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); setNotice('Copied to clipboard.') }
    catch { setNotice('Could not copy automatically. Select the output and copy it manually.') }
  }

  const descriptions = { Encode: 'Replace sensitive information with tokens.', Decode: 'Restore tokens to their original values.', Vault: 'Inspect and edit the values your tokens restore to.' }

  return (
    <div className={`app-shell${light ? ' light' : ''}`}>
      <aside className="sidebar">
        <button className="brand" onClick={() => navigate('Encode')} aria-label="Poco home">
          <span className="logo-frame"><img src={`${import.meta.env.BASE_URL}poco_logo.png`} alt="" /></span>
          <span>Poco</span>
        </button>
        <nav aria-label="Main navigation">
          {(['Encode', 'Decode', 'Vault'] as const).map((item) => (
            <button key={item} className={`nav-item${page === item ? ' active' : ''}`} aria-current={page === item ? 'page' : undefined} onClick={() => navigate(item)}>
              <NavIcon page={item} /><span><strong>{item}</strong><small>{{ Encode: 'Protect sensitive text', Decode: 'Restore original text', Vault: 'Inspect stored mappings' }[item]}</small></span>
            </button>
          ))}
        </nav>
        <p className="demo-note">Local demo · sample values only.<br />Documents clear on refresh.</p>
      </aside>
      <div className="main-shell">
        <main>
          <div className="page-heading"><div><h1>{page}</h1><p>{descriptions[page]}</p></div>
            <div className="page-actions">
            {page === 'Vault' && <button className="danger" disabled={!active.mappings.length} onClick={() => { changeMappings([]); setNotice('This document’s vault has been reset.') }}>Reset vault</button>}
              <button className="theme-button" onClick={() => setLight(!light)} aria-label={`Switch to ${light ? 'dark' : 'light'} theme`}>
          <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><circle cx="12" cy="12" r="4" /><path d="M12 1v2m0 18v2M1 12h2m18 0h2M4 4l2 2m12 12 2 2M4 20l2-2M18 6l2-2" /></svg>
        </button>
            </div>
          </div>
          <div className="document-tabs" role="group" aria-label="Documents">
            {documents.map((doc) => <button key={doc.id} aria-pressed={doc.id === activeId} onClick={() => { setActiveId(doc.id); setNotice('') }}>{doc.name}</button>)}
            <button className="new-document" onClick={addDocument}>+ New document</button>
          </div>

          {page !== 'Vault' && <>
            <div className="editors">
              <section className="editor">
                <div className="editor-heading">
                  <label htmlFor="source-input">Input</label>
                  <div>
                    <button className="quiet" onClick={() => fileInput.current?.click()}>Upload file</button>
                    <button className="quiet" onClick={() => {
                      updateDocument(activeId, encoding
                        ? { input: sampleText, name: 'onboarding-dossier.txt', output: '' }
                        : { reply: sampleReply, restored: '', unknown: [] })
                      setNotice(encoding ? 'Example loaded.' : 'Encode the example in this document first to create its mappings.')
                    }}>Use example</button>
                    <input ref={fileInput} type="file" accept=".txt,.md,text/plain,text/markdown" className="sr-only" tabIndex={-1} aria-label="Upload text file" onChange={(e) => { void upload(e.target.files?.[0]); e.target.value = '' }} />
                  </div>
                </div>
                <textarea
                  id="source-input"
                  value={input}
                  onChange={(e) => {
                    updateDocument(activeId, encoding
                      ? { input: e.target.value, output: '' }
                      : { reply: e.target.value, restored: '', unknown: [] })
                    setNotice('')
                  }}
                  placeholder={encoding ? 'Paste text here or upload a file…' : 'Paste text with tokens here or upload a file…'}
                  spellCheck={false}
                />
              </section>
              <section className="editor">
                <div className="editor-heading">
                  <h2 id="output-label">Output</h2>
                  <button className="quiet" disabled={!output} onClick={() => copy(output)}>Copy</button>
                </div>
                <pre className="output" tabIndex={0} aria-labelledby="output-label">{output
                  ? encoding ? <TokenText text={output} mappings={active.mappings} /> : output
                  : <span className="placeholder">{encoding ? 'Run Encode to see tokenized output.' : 'Run Decode to see restored text.'}</span>}</pre>
              </section>
            </div>
            <div className="actions">
              <button className="primary" disabled={!input.trim()} onClick={encoding ? encode : decode}>{page}</button>
              <button className="secondary" onClick={() => navigate('Vault')}>Inspect PII</button>
            </div>
            {!encoding && !!active.unknown.length && <p className="warning" role="alert">Unknown tokens left unchanged: {active.unknown.join(', ')}</p>}
          </>}

          {page === 'Vault' && <Vault key={activeId} mappings={active.mappings} onChange={changeMappings} onAdd={() => {
            const mapping = { token: `Custom_${active.nextToken}`, value: 'new value', category: 'Custom', firstOffset: 0 }
            updateDocument(activeId, { mappings: [...active.mappings, mapping], nextToken: active.nextToken + 1, restored: '', unknown: [] })
            setNotice('Token added. Choose Edit to change its value and class.')
          }} />}
          <p className="status" role="status">{notice}</p>
        </main>
      </div>
    </div>
  )
}

export default App
