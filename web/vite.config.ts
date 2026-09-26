import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  // Relative base so assets work on GitHub Pages project sites
  // (https://user.github.io/REPO/) and locally.
  base: './',
  plugins: [react()],
})
