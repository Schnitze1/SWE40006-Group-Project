export type Mapping = {
  token: string
  value: string
  category: string
  firstOffset: number
}

export const sampleText = `Please contact Sarah Mitchell about her onboarding.

Email: sarah@example.com
Phone: +61 412 345 678

Sarah Mitchell would like a confirmation by email.`

export const sampleReply = 'I will contact [Name_1] at [Email_1] and call [Phone_1] to confirm the details. Reference: [Person_99].'

const examples = [
  { token: 'Name_1', value: 'Sarah Mitchell', category: 'Name' },
  { token: 'Email_1', value: 'sarah@example.com', category: 'Email' },
  { token: 'Phone_1', value: '+61 412 345 678', category: 'Phone' },
]

// Deliberately a fixture demonstration, not a PII detector.
export function encodeDemo(text: string) {
  const mappings: Mapping[] = examples
    .map((entry) => ({ ...entry, firstOffset: text.indexOf(entry.value) }))
    .filter((entry) => entry.firstOffset >= 0)
    .sort((a, b) => a.firstOffset - b.firstOffset)
  let redactedText = text
  for (const entry of mappings) {
    redactedText = redactedText.split(entry.value).join(`[${entry.token}]`)
  }
  return { redactedText, mappings }
}

export function decodeDemo(text: string, mappings: Mapping[]) {
  const hallucinations: string[] = []
  const restoredText = text.replace(/\[([A-Za-z][A-Za-z0-9_:]*_\d+)\]/g, (placeholder, token: string) => {
    const mapping = mappings.find((entry) => entry.token === token)
    if (mapping) return mapping.value
    if (!hallucinations.includes(placeholder)) hallucinations.push(placeholder)
    return placeholder
  })
  return { restoredText, hallucinations }
}
