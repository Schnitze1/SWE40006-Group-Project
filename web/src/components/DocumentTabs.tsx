import type { DocTab } from '../types'

type Props = {
  documents: DocTab[]
  activeId: number
  onSelect: (id: number) => void
  /** Close is never shown when documents.length === 1. */
  onClose: (id: number) => void
  onAdd: () => void
}

/**
 * Shared tab strip (one implementation for Encode and Vault).
 * Close (×) only renders when more than one document exists.
 */
export function DocumentTabs({ documents, activeId, onSelect, onClose, onAdd }: Props) {
  const canClose = documents.length > 1

  return (
    <div className="doctabs" role="group" aria-label="Documents">
      {documents.map((doc) => (
        <div
          key={doc.id}
          className={`doctab${doc.id === activeId ? ' active' : ''}`}
          onClick={(e) => {
            // Case 1: click landed on the close control.
            const target = e.target as HTMLElement
            if (target.dataset.del) {
              e.stopPropagation()
              onClose(doc.id)
              return
            }
            // Case 2: normal tab click — switch this view only.
            onSelect(doc.id)
          }}
        >
          <span className="doctab-name">{doc.name}</span>
          {canClose && (
            <span
              className="doctab-x"
              data-del={doc.id}
              role="button"
              aria-label={`Close ${doc.name}`}
              title="Close document"
            >
              ×
            </span>
          )}
        </div>
      ))}
      <div className="doctab add" onClick={onAdd} role="button" tabIndex={0}
        onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onAdd() } }}>
        + New document
      </div>
    </div>
  )
}
