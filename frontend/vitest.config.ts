import { defineConfig } from 'vitest/config'
import solid from 'vite-plugin-solid'

/**
 * Kept separate from `vite.config.ts` on purpose.
 *
 * Solid components have to be compiled for tests the same way they are for the
 * app, but `resolve.conditions` below asks for Solid's *development* build —
 * which is what makes `render` and reactivity work under a test runner, and is
 * exactly what a production bundle must not have. Two files means the test
 * setup cannot leak into what `pnpm build` ships.
 *
 * Tailwind is absent for the same reason it is not needed: nothing here asserts
 * on styling, and compiling the stylesheet would only make the suite slower.
 */
export default defineConfig({
  // `hot: false` disables solid-refresh. It exists to preserve component state
  // across an edit in the dev server, which a test run has no use for, and
  // injecting it here fails outright: the transform emits an import of
  // `/@solid-refresh`, a dev-server virtual module that nothing resolves under
  // the test runner.
  plugins: [solid({ hot: false })],
  resolve: {
    conditions: ['development', 'browser'],
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.{ts,tsx}'],
    setupFiles: ['./src/test-setup.ts'],
    // Solid's reactive graph is module-level state; a shared environment
    // between files would let one test's cleanup affect another's.
    isolate: true,
  },
})
