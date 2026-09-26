# Poco UI demo

React + TypeScript + Vite frontend. Run these commands from `web/`:

```sh
npm ci
npm run dev
```

`npm run build` checks TypeScript and creates `dist/`. `npm run preview` serves that build locally. `npm run lint` runs Oxlint.

## Try the demo

1. In **Encode**, choose **Use example**, then **Encode**. Tokens are highlighted in the output.
2. Choose **Inspect PII** or **Vault** to filter, edit, add, or delete mappings. **Reset vault** clears the selected document's mappings.
3. In **Decode**, choose **Use example**, then **Decode**. Known tokens restore from the selected document's vault; `[Person_99]` intentionally stays unknown.
4. **New document** creates a separate input, output, and vault. Switching documents or views preserves work until refresh.
5. **Upload file** reads local TXT/MD files up to 1 MB. The theme button switches between dark and light.

Encoding creates a fresh mapping snapshot for the selected document, replacing its vault edits. Editing a mapping changes future decoding without changing its token identifier. No data is persisted across refreshes.

Only three predefined values in `src/mocks/demo.ts` are replaced. This is not a PII detector and is not connected to Rust, AWS, or an LLM. No input or mapping data is persisted or sent to a service. PDF/DOCX upload is not implemented.

The mock result fields follow the relevant shapes in `../docs/API.md`; a future integration must add API calls and session IDs.

## Git

Commit source, configuration, `package.json`, and `package-lock.json`. Ignore `node_modules/`, `dist/`, local environment files, logs, coverage, and TypeScript build metadata. `.env.example` can be committed with placeholder values only.