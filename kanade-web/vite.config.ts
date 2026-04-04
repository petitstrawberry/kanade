import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'path';

export default defineConfig({
  plugins: [svelte()],
  publicDir: 'static',
  resolve: {
    extensions: ['.svelte.ts', '.ts', '.svelte', '.js'],
  },
});
