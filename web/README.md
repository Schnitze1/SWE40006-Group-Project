# Poco UI demo

React + TypeScript + Vite frontend. Run these commands from `web/`:

```sh
npm ci
npm run dev
```

`npm run build` checks TypeScript and creates `dist/`. `npm run preview` serves that build locally. `npm run lint` runs Oxlint.

## Try the demo

1. Choose **Encode**, then **Try a sample** and **Encode text**.
2. Review the output and session mappings; use **Copy** if desired.
3. Choose **Decode**, load the sample response, and restore it.
4. Known tokens restore their sample values. `[Person_99]` is intentionally unknown and stays unchanged.
5. **Clear session** removes input, output, and mappings. Refreshing also resets the session.

Only three predefined values in `src/mocks/demo.ts` are replaced. This is not a PII detector and is not connected to Rust, AWS, or an LLM. No input or mapping data is persisted or sent to a service. PDF/DOCX upload is not implemented.

The mock result fields follow the relevant shapes in `../docs/API.md`; a future integration must add API calls and session IDs.

## Git

Commit source, configuration, `package.json`, and `package-lock.json`. Ignore `node_modules/`, `dist/`, local environment files, logs, coverage, and TypeScript build metadata. `.env.example` can be committed with placeholder values only.
