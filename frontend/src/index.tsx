/* @refresh reload */
import { render } from 'solid-js/web'
import { Route, Router } from '@solidjs/router'

import './index.css'
import Layout from './components/Layout'
import Dashboard from './routes/Dashboard'
import PagesBrowse from './routes/PagesBrowse'
import PageDetail from './routes/PageDetail'
import Tags from './routes/Tags'
import NotFound from './routes/NotFound'

const root = document.getElementById('root')

render(
  () => (
    <Router root={Layout}>
      <Route path="/" component={Dashboard} />
      <Route path="/pages" component={PagesBrowse} />
      {/*
        A splat, not `:slug`. Slugs contain `/` — `notes/rust/async` is one
        slug, not three segments — so this route has to swallow the rest of
        the path. Matching verified against `@solidjs/router`'s own matcher:
        `/pages/notes/rust/async` yields `{ slug: "notes/rust/async" }`, and
        the literal `/pages` route still wins for the bare listing.
      */}
      <Route path="/pages/*slug" component={PageDetail} />
      <Route path="/tags" component={Tags} />
      <Route path="/*404" component={NotFound} />
    </Router>
  ),
  root!,
)
