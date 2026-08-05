import { createEffect } from 'solid-js'
import { useNavigate } from '@solidjs/router'
import { PAGE_ROUTE_PREFIX } from '../api/client'

/**
 * Show HTML that the server rendered.
 *
 * This is the only place in the app that assigns `innerHTML`, and it is safe
 * for one specific reason: the string is comrak's output with raw HTML turned
 * off, so markup written into a page body is dropped by the renderer rather
 * than passed through. Nothing else may be routed through here. A page's
 * markdown source and a search snippet are both *unescaped* text straight out
 * of the wiki, and rendering either as HTML would throw away the guarantee the
 * server is making on our behalf.
 */
export default function Markdown(props: { html: string; class?: string }) {
  const navigate = useNavigate()
  let container!: HTMLDivElement

  createEffect(() => {
    container.innerHTML = props.html

    // Anything leaving the wiki opens away from the app. Nothing inside a page
    // body should be able to navigate the dashboard's own tab off somewhere.
    for (const anchor of container.querySelectorAll('a[href]')) {
      if (isPageLink(anchor.getAttribute('href'))) continue
      anchor.setAttribute('target', '_blank')
      anchor.setAttribute('rel', 'noopener noreferrer')
    }
  })

  /**
   * Keep links between pages inside the SPA, so following one is instant
   * instead of a full reload.
   *
   * Delegated from the container rather than bound to each anchor: the anchors
   * are plain DOM that this component replaces wholesale, and a listener on the
   * container outlives that.
   */
  const onClick = (event: MouseEvent) => {
    if (event.defaultPrevented || event.button !== 0) return
    // A modified click means "open this somewhere else". Leave it to the
    // browser, which already does exactly the right thing.
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return

    const href = (event.target as Element | null)?.closest('a')?.getAttribute('href')
    if (!isPageLink(href)) return

    event.preventDefault()
    navigate(href)
  }

  return <div ref={container} class={props.class} onClick={onClick} />
}

/**
 * Whether an href names a page in this wiki.
 *
 * The server emits these as root-absolute `/pages/...` URLs precisely so that
 * recognising them is a prefix check rather than a re-implementation of the
 * backend's link resolution.
 */
function isPageLink(href: string | null | undefined): href is string {
  return typeof href === 'string' && href.startsWith(PAGE_ROUTE_PREFIX)
}
