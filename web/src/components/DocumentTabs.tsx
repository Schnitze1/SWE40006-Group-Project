import { useState } from 'react'

export type DocTab = {
  id: number
  name: string
}

type Props = {
  documents: DocTab[]
  activeId: number
  onSelect: (id: number) => void
  onClose: (id: number) => void
  onAdd: () => void
}

/** Shared tab strip for Encode and Vault. Close is hidden when only one doc. */
export function DocumentTabs({ documents, activeId, onSelect, onClose, onAdd }: Props) {
  const [confirmId, setConfirmId] = useState<number | null>(null)
  const canClose = documents.length > 1

  return (
    <div className="document-tabs" role="group" aria-label="Documents">
      {documents.map((doc) => (
        <div
          key={doc.id}
          className={`document-tab${doc.id === activeId ? ' active' : ''}${confirmId === doc.id ? ' confirming' : ''}`}
        >
          <button
            type="button"
            className="document-tab-label"
            aria-pressed={doc.id === activeId}
            onClick={() => {
              setConfirmId(null)
              onSelect(doc.id)
            }}
          >
            {doc.name}
          </button>
          {canClose && (
            <button
              type="button"
              className="document-tab-close"
              aria-label={confirmId === doc.id ? `Confirm close ${doc.name}` : `Close ${doc.name}`}
              title={confirmId === doc.id ? 'Click again to close' : 'Close document'}
              onClick={(e) => {
                e.stopPropagation()
                if (confirmId === doc.id) {
                  setConfirmId(null)
                  onClose(doc.id)
                } else {
                  setConfirmId(doc.id)
                  window.setTimeout(() => setConfirmId((cur) => (cur === doc.id ? null : cur)), 3000)
                }
              }}
            >
              ×
            </button>
          )}
          {confirmId === doc.id && (
            <span className="document-tab-flash" role="status">
              Close?
            </span>
          )}
        </div>
      ))}
      <button type="button" className="new-document" onClick={onAdd}>
        + New document
      </button>
    </div>
  )
}
