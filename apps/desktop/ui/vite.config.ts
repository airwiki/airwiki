import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { strictPort: true },
  build: { target: ['es2022', 'chrome105', 'safari15'] }
});
