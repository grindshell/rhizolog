/* @refresh reload */
import { render } from 'solid-js/web'
import { Route, Router } from '@solidjs/router'

import './index.css'
import Layout from './components/Layout'
import Dashboard from './routes/Dashboard'
import PagesBrowse from './routes/PagesBrowse'
import PageDetail from './routes/PageDetail'
import Editor from './routes/Editor'
import Tags from './routes/Tags'
import Times from './routes/Times'
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
      {/*
        Editing lives outside `/pages` for the same reason the backend's move
        endpoint does: a splat has to be the last thing in the path, so
        `/pages/*slug/edit` cannot be expressed, and a literal `/pages/edit`
        would shadow any page actually slugged `edit`.
      */}
      <Route path="/new" component={Editor} />
      <Route path="/edit/*slug" component={Editor} />
      <Route path="/tags" component={Tags} />
      {/*
        No `/times/:id` route. A time entry is read and edited in the log
        itself, and an id is a machine's handle rather than something anyone
        would link to — unlike a slug, which is the whole point of a page.
      */}
      <Route path="/times" component={Times} />
      <Route path="/*404" component={NotFound} />
    </Router>
  ),
  root!,
)
