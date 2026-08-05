/**
 * jsdom implements no layout, so it has no `scrollTo` — and `@solidjs/router`
 * calls it on every navigation to restore scroll position. Left alone it prints
 * a "Not implemented" error for each one, which buries anything that actually
 * matters in the test output.
 */
window.scrollTo = () => {}
