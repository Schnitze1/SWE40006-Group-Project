/**
 * Client-side PDF text extraction.
 * Large multi-page PDFs hit API Gateway / Lambda payload caps (HTTP 413);
 * extracting in the browser and sending only text avoids that entirely.
 */
export async function extractPdfText(file: File): Promise<string> {
  const pdfjs = await import('pdfjs-dist')
  // Vite serves the worker as a module URL.
  const workerUrl = new URL('pdfjs-dist/build/pdf.worker.min.mjs', import.meta.url)
  pdfjs.GlobalWorkerOptions.workerSrc = workerUrl.href

  const buf = await file.arrayBuffer()
  const doc = await pdfjs.getDocument({ data: buf }).promise
  const parts: string[] = []
  for (let i = 1; i <= doc.numPages; i++) {
    const page = await doc.getPage(i)
    const content = await page.getTextContent()
    const line = content.items
      .map((it) => ('str' in it ? it.str : ''))
      .join('')
    parts.push(line)
  }
  return parts.join('\n')
}
