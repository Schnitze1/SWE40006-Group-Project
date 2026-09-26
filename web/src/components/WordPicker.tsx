import { useMemo } from 'react'

type Props = {
  value: string
  onPick: (word: string) => void
  /** Rendered over the textarea for hover/click word picking. */
  placeholder?: string
}

/**
 * Transparent word layer over the input: hover highlights a word,
 * click creates a vault token draft from that word.
 */
export function WordPicker({ value, onPick, placeholder }: Props) {
  const parts = useMemo(() => {
    // Keep whitespace/newlines so the layer wraps like the textarea.
    return value.split(/(\s+)/).filter((p) => p.length > 0)
  }, [value])

  if (!value.trim()) {
    return (
      <div className="word-picker empty" aria-hidden="true">
        {placeholder ?? ''}
      </div>
    )
  }

  return (
    <div className="word-picker" aria-hidden="true">
      {parts.map((part, i) => {
        if (/^\s+$/.test(part)) {
          return (
            <span key={i} className="word-space">
              {part}
            </span>
          )
        }
        return (
          <span
            key={i}
            className="word-pick"
            title="Click to create a token from this word"
            onClick={(e) => {
              e.preventDefault()
              e.stopPropagation()
              onPick(part)
            }}
          >
            {part}
          </span>
        )
      })}
    </div>
  )
}
