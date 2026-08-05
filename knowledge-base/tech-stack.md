# Tech stack

## Backend — Rust (`backend/`)

- **tokio** — async executor
- **axum** — HTTP server
- **utoipa-axum** — OpenAPI definitions, kept in sync with the axum routes

## Frontend — TypeScript (`frontend/`)

- **pnpm 9** — package manager
- **Vite 8** + **SolidJS 1.9** — build tool and framework
- **@solidjs/router 1** — routing
- **TailwindCSS 4** + **daisyUI 5** — styling
- **@tailwindcss/typography** — styles for rendered page bodies
- **openapi-typescript 7** — generates request/response types from the API

The frontend is served by the backend; there is no separate frontend
deployment. What it actually does is in [The dashboard](dashboard.md).

Notably absent: any markdown or editor library. The server renders, so the
client does not need to — see the note on `POST /api/render` in
[API design](api-design.md).

### Tailwind 4 has no config file

Tailwind resolved to v4, whose setup is not the v3 setup most guides describe.
There is **no `tailwind.config.js`, no `postcss.config.js`, and no `content`
globs** — `@tailwindcss/vite` is a Vite plugin and content detection is
automatic. daisyUI 5 loads through Tailwind's CSS `@plugin` directive rather
than a JS plugin array, so the whole configuration is `src/index.css`:

```css
@import 'tailwindcss';

@plugin 'daisyui' {
  themes: light --default, dark --prefersdark;
}
```

Worth knowing before following a v3-era tutorial and concluding the install is
broken.

The typography plugin loads the same way. It is not decoration: Tailwind's
preflight strips the browser's default styling from headings, lists, and
blockquotes, so without it a rendered markdown page is correct HTML that reads as
one flat wall of text.

### Slugs are a splat route here too

`/pages/*slug` — the same constraint that shaped the backend's routes. A slug
is one parameter containing slashes, not several path segments.

One trap on top of that: `@solidjs/router` reads `location.pathname` verbatim
and does **not** percent-decode path parameters, so `notes/my%20page` arrives
still encoded. The client's `decodeSlug` handles it per segment; anything
reading the slug param has to go through that.

### The backend needs an SPA fallback

`/pages/notes/rust/async` is a client route with no file behind it. The backend
serves `frontend/dist` with a fallback to `index.html` so a pasted link or a
hard refresh works — otherwise pages would only be reachable by navigating from
inside the app.

Unknown `/api` paths are excluded from that fallback and still return the JSON
error envelope. Answering a mistyped endpoint with a 200 of HTML would be a
particularly confusing thing to do to an agent.

`RHIZOWIKI_ASSETS` overrides the directory. A missing build is not an error:
during frontend work `pnpm dev` serves the UI and proxies `/api`,
`/api-docs`, and `/swagger-ui` to the backend.
