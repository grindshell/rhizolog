// @ts-check
import { defineConfig } from 'astro/config';
import tailwindcss from '@tailwindcss/vite';

/*
 * Tailwind v4 loads through its Vite plugin rather than an Astro integration,
 * which is how `frontend/` does it too. There is no `tailwind.config.js` in
 * either place: the theme lives in `src/styles/global.css` under `@theme`.
 */
export default defineConfig({
  site: 'https://rhizolog.com',
  vite: {
    plugins: [tailwindcss()],
  },
});
