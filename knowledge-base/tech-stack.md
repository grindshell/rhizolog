# Tech stack

## Backend — Rust (`app/`)

- **tokio** — async executor
- **axum** — HTTP server
- **utoipa-axum** — OpenAPI definitions, kept in sync with the axum routes

## Frontend — TypeScript (`app/frontend/`, not yet scaffolded)

- **pnpm** — package manager
- **SolidJS** — frontend framework
- **TailwindCSS** — CSS framework
- **DaisyUI** — component library

The frontend is served by the backend; there is no separate frontend
deployment.
