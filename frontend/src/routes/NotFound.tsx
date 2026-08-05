import { A } from '@solidjs/router'

export default function NotFound() {
  return (
    <div class="hero py-16">
      <div class="hero-content text-center">
        <div>
          <h1 class="text-3xl font-semibold">No such screen</h1>
          <p class="py-4 opacity-70">
            That is a route the dashboard does not have. Page slugs live under{' '}
            <code class="font-mono">/pages/</code>.
          </p>
          <A class="btn btn-primary" href="/">
            Back to the dashboard
          </A>
        </div>
      </div>
    </div>
  )
}
