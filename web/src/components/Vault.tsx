import { useState } from 'react'
import type { Mapping } from '../api'

const categories = ['Name', 'Email', 'Phone', 'Location', 'Date', 'Passport', 'Ssn', 'Iban', 'CreditCard', 'Organization', 'Custom']

type Props = {
  mappings: Mapping[]
  onChange: (mappings: Mapping[]) => void
  onAdd: () => void
}

export function Vault({ mappings, onChange, onAdd }: Props) {
  const [filter, setFilter] = useState('')
  const [editing, setEditing] = useState<string | null>(null)
  const [value, setValue] = useState('')
  const [category, setCategory] = useState('Custom')
  const visible = mappings.filter((m) => `${m.token} ${m.value} ${m.category}`.toLowerCase().includes(filter.toLowerCase()))

  function save() {
    if (!value.trim()) return
    onChange(mappings.map((m) => m.token === editing ? { ...m, value, category } : m))
    setEditing(null)
  }

  return (
    <>
      <div className="filter-bar">
        <input aria-label="Filter mappings" placeholder="Filter by token, value, or class" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <span>{visible.length} of {mappings.length} mappings</span>
      </div>
      <div className="table-scroll">
        <table>
          <thead><tr><th scope="col">Token</th><th scope="col">Value</th><th scope="col">Class</th><th scope="col"><span className="sr-only">Actions</span></th></tr></thead>
          <tbody>
            {visible.map((m) => (
              <tr key={m.token}>
                <td><code className={`token token-${m.category.toLowerCase()}`}>[{m.token}]</code></td>
                <td>{editing === m.token
                  ? <input autoFocus aria-label={`Value for ${m.token}`} value={value} onChange={(e) => setValue(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') save(); if (e.key === 'Escape') setEditing(null) }} />
                  : m.value}</td>
                <td>{editing === m.token
                  ? <select aria-label={`Class for ${m.token}`} value={category} onChange={(e) => setCategory(e.target.value)}>{categories.map((c) => <option key={c}>{c}</option>)}</select>
                  : <span className={`class-badge token-${m.category.toLowerCase()}`}>{m.category}</span>}</td>
                <td><div className="row-actions">
                  {editing === m.token ? <>
                    <button className="quiet" onClick={save} disabled={!value.trim()} aria-label={`Save ${m.token}`}>Save</button>
                    <button className="quiet" onClick={() => setEditing(null)}>Cancel</button>
                  </> : <button className="quiet" aria-label={`Edit ${m.token}`} onClick={() => { setEditing(m.token); setValue(m.value); setCategory(m.category) }}>Edit</button>}
                  <button className="quiet delete" aria-label={`Delete ${m.token}`} onClick={() => { onChange(mappings.filter((entry) => entry.token !== m.token)); if (editing === m.token) setEditing(null) }}>Delete</button>
                </div></td>
              </tr>
            ))}
            {!visible.length && <tr><td colSpan={4} className="empty-state">{mappings.length ? 'No matching tokens.' : 'No tokens yet. Encode an example or add a token.'}</td></tr>}
          </tbody>
        </table>
      </div>
      <div className="add-token"><button className="quiet" onClick={() => { setFilter(''); onAdd() }}>+ Add token</button></div>
    </>
  )
}
