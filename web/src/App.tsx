import { useState } from 'react'
import { decodeDemo, encodeDemo, sampleReply, sampleText } from './mocks/demo'
import type { Mapping } from './mocks/demo'
import './App.css'

type Mode = 'encode' | 'decode'

function App() {
  const [mode, setMode] = useState<Mode>('encode')
  const [input, setInput] = useState('')
  const [output, setOutput] = useState('')
  const [mappings, setMappings] = useState<Mapping[]>([])
  const [unknown, setUnknown] = useState<string[]>([])
  const [notice, setNotice] = useState('')
  const encoding = mode === 'encode'

  function resetResult() {
    setOutput('')
    setUnknown([])
    setNotice('')
  }

  function switchMode(next: Mode) {
    if (next === mode) return
    setMode(next)
    setInput('')
    resetResult()
  }

  function runDemo() {
    resetResult()
    if (encoding) {
      const result = encodeDemo(input)
      setOutput(result.redactedText)
      setMappings((previous) => {
        const additions = result.mappings.filter((entry) => !previous.some((old) => old.token === entry.token))
        return [...previous, ...additions]
      })
      setNotice(result.mappings.length
        ? `Replaced ${result.mappings.length} sample values. Other personal information is not detected.`
        : 'No predefined sample values matched. This demo does not detect other personal information.')
    } else {
      const result = decodeDemo(input, mappings)
      setOutput(result.restoredText)
      setUnknown(result.hallucinations)
      setNotice('Demo response restored using the current session mappings.')
    }
  }

  async function copyOutput() {
    try {
      await navigator.clipboard.writeText(output)
      setNotice('Result copied to clipboard.')
    } catch {
      setNotice('Could not copy automatically. Select and copy the result below.')
    }
  }

  function clearSession() {
    setInput('')
    setMappings([])
    resetResult()
    setNotice('Session cleared.')
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <a className="brand" href="./">
          <span className="brand-mark" aria-hidden="true">[p]</span>
          Poco
        </a>
        <span className="demo-badge">Demo</span>
      </header>

      <main>
        <div className="intro">
          <h1>Personal details. Made private.</h1>
          <p>Replace details with tokens. Restore them when you’re ready.</p>
        </div>

        <section className="workspace" aria-label="Text workspace">
          <div className="workspace-toolbar">
            <div className="mode-switch" role="group" aria-label="Choose workflow">
              <button aria-pressed={encoding} onClick={() => switchMode('encode')}>Encode</button>
              <button aria-pressed={!encoding} onClick={() => switchMode('decode')}>Decode</button>
            </div>
            <button className="text-button" onClick={clearSession}>Clear session</button>
          </div>

          <div className="panels">
            <div className="text-panel">
              <div className="panel-heading">
                <label htmlFor="source-text">{encoding ? 'Your text' : 'Response with tokens'}</label>
                <button className="text-button" onClick={() => {
                  setInput(encoding ? sampleText : sampleReply)
                  resetResult()
                }}>Try a sample</button>
              </div>
              <textarea
                id="source-text"
                value={input}
                onChange={(event) => { setInput(event.target.value); resetResult() }}
                placeholder={encoding
                  ? 'Paste your text here…'
                  : 'Encode the sample first, then paste a response with tokens such as [Name_1].'}
                spellCheck={false}
              />
            </div>
            <div className="text-panel result-panel">
              <div className="panel-heading">
                <label htmlFor="result-text">{encoding ? 'Encoded text' : 'Restored text'}</label>
                <button className="text-button" disabled={!output} onClick={copyOutput}>Copy</button>
              </div>
              <textarea
                id="result-text"
                value={output}
                readOnly
                placeholder="Your result will appear here."
              />
            </div>
          </div>

          <div className="workspace-footer">
            <span>Sample values only · No data is sent</span>
            <button className="primary-button" disabled={!input.trim()} onClick={runDemo}>
              {encoding ? 'Encode text' : 'Restore text'} <span aria-hidden="true">→</span>
            </button>
          </div>
        </section>

        <p className="status" role="status">{notice}</p>
        {unknown.length > 0 && (
          <aside className="warning" role="alert">
            <strong>Unknown tokens</strong>
            <p>Left unchanged because they have no mapping: {unknown.join(', ')}</p>
          </aside>
        )}

        <section className="mapping-section" aria-labelledby="mapping-title">
          <div className="mapping-heading">
            <h2 id="mapping-title">Session mappings <span className="count">{mappings.length}</span></h2>
            <span>Cleared when you refresh</span>
          </div>
          {mappings.length === 0 ? (
            <p className="empty-state">Encode the sample to see its tokens and original values here.</p>
          ) : (
            <div className="table-scroll">
              <table>
                <thead><tr><th scope="col">Token</th><th scope="col">Original value</th><th scope="col">Type</th></tr></thead>
                <tbody>
                  {mappings.map((mapping) => (
                    <tr key={mapping.token}>
                      <td><code>[{mapping.token}]</code></td>
                      <td>{mapping.value}</td>
                      <td className="category">{mapping.category}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
        <p className="demo-note">UI demo. Only the sample name, email and phone are replaced.</p>
      </main>
    </div>
  )
}

export default App
