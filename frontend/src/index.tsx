/* @refresh reload */
import { render } from 'solid-js/web'
import { Route, Router } from '@solidjs/router'

import './index.css'
import Layout from './components/Layout'
import SessionGate from './components/SessionGate'
import Accounts from './routes/Accounts'
import Dashboard from './routes/Dashboard'
import PagesBrowse from './routes/PagesBrowse'
import PageDetail from './routes/PageDetail'
import Editor from './routes/Editor'
import GraphView from './routes/GraphView'
import Inbox from './routes/Inbox'
import Ideas from './routes/Ideas'
import IdeaDetail from './routes/IdeaDetail'
import Tags from './routes/Tags'
import Times from './routes/Times'
import NotFound from './routes/NotFound'

const root = document.getElementById('root')

render(
  () => (
    /*
      Outside the router, not a `/login` route. A redirect to one would throw
      away the address somebody arrived at, and in this app an address is a page
      — a link to `/pages/notes/rust/async` should still open that page once its
      recipient has signed in. The gate swaps the whole router for a sign-in
      form and back, leaving the URL alone.

      On a wiki with no accounts it renders its children and nothing else.
    */
    <SessionGate>
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
          `/graph`, and the page it is drawn around is `?root=` rather than a
          path segment. A root is a filter like `?prefix=` and `?tag=` beside it,
          composes with them, and is often absent — and a splat here would have
          made `/graph` itself a different route from `/graph/notes/rust/async`.
        */}
        <Route path="/graph" component={GraphView} />
        {/*
          No `/times/:id` route. A time entry is read and edited in the log
          itself, and an id is a machine's handle rather than something anyone
          would link to — unlike a slug, which is the whole point of a page.
        */}
        <Route path="/times" component={Times} />
        {/*
          `:id` rather than a splat, and no `/captures/:id` beside it. An idea
          id has no slashes in it, so an ordinary path parameter is enough.
          A capture is read where it lives, in the inbox and in whatever
          threads hold it, because it is working material rather than a
          document anybody would link somebody else to.
        */}
        <Route path="/inbox" component={Inbox} />
        <Route path="/ideas" component={Ideas} />
        <Route path="/ideas/:id" component={IdeaDetail} />
        {/*
          A route rather than a section of the shell, because on an open wiki it
          is where the first account gets created — and there is no account menu
          to reach it from until one exists.
        */}
        <Route path="/accounts" component={Accounts} />
        <Route path="/*404" component={NotFound} />
      </Router>
    </SessionGate>
  ),
  root!,
)
