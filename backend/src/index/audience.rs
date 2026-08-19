//! One predicate, applied everywhere a page can be seen.
//!
//! Visibility that is enforced in the page-reading handler and nowhere else is
//! not visibility. A private page still shows up in the listing, in search
//! results with an excerpt of its body, as a backlink with its title, in the
//! tag histogram, in the graph, and in `most_linked` — each of which leaks a
//! different piece of it. So the rule lives here, as **one SQL fragment**, and
//! every query that can return a page pastes it in.
//!
//! That is deliberately the boring, repetitive answer. The alternatives — a view
//! over `pages`, or filtering in Rust after the query — each fail in a way that
//! matters: a view cannot be parameterised by who is asking without a session
//! variable SQLite does not have, and filtering afterwards breaks `count(*)`,
//! `limit`/`offset` and every aggregate, which is how a paginated listing ends
//! up with pages that are silently short.
//!
//! ## Named parameters, because the numbers would drift
//!
//! The queries this is pasted into already bind `?1`..`?5`. Adding two more
//! positional parameters to each of them means renumbering by hand at every call
//! site, which is the kind of edit that compiles and returns the wrong rows. The
//! touched queries therefore use `:name` throughout — SQLite is happy to mix the
//! two, but only one of them can be got wrong silently.

use rusqlite::ToSql;
use rusqlite::types::Null;

use crate::users::Username;

/// What a caller is allowed to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Audience {
    /// Everything, unconditionally. This wiki has no accounts, so it has no
    /// visibility either — there is nobody to keep a page from.
    Everything,
    /// Public pages and nothing else. A caller that has not signed in, on an
    /// instance that has opted into serving those at all.
    Public,
    /// Public and internal pages, plus this account's own and anything
    /// restricted to it.
    Account(Username),
}

impl Audience {
    /// Whether this audience is allowed to see anything beyond public pages.
    pub fn is_signed_in(&self) -> bool {
        matches!(self, Self::Everything | Self::Account(_))
    }

    /// The bindings the fragment below expects.
    ///
    /// Returned together with the SQL so the two cannot be updated apart. Both
    /// are always bound, whichever variant this is, because a prepared statement
    /// with an unbound named parameter is an error rather than a NULL.
    pub fn params(&self) -> [(&'static str, &dyn ToSql); 2] {
        match self {
            Self::Everything => [(":everything", &1_i64), (":viewer", &Null)],
            Self::Public => [(":everything", &0_i64), (":viewer", &Null)],
            Self::Account(username) => [(":everything", &0_i64), (":viewer", username)],
        }
    }
}

/// True for a page the audience may read.
///
/// Written against the bare column names, so it works in any query where
/// `pages` is in scope under its own name. The four clauses are the ladder in
/// [`crate::page::Visibility`] plus the one rule that cuts across it: **your own
/// pages are always yours**, whatever their visibility says, so a page you
/// marked private does not disappear from you.
///
/// A `private` page with no owner is readable by nobody. That is the safe
/// direction for the field to fail in, and it is why writing one through the API
/// fills the owner in.
pub const VISIBLE: &str = "(
    :everything = 1
    or pages.visibility = 'public'
    or (:viewer is not null and pages.visibility = 'internal')
    or (:viewer is not null and pages.owner = :viewer)
    or (:viewer is not null and pages.visibility = 'restricted' and exists (
            select 1 from page_readers
            where page_readers.slug = pages.slug
              and page_readers.username = :viewer
        ))
)";

/// The same rule for a query that has joined `pages` under an alias.
///
/// Only `links` needs it, where a page appears twice in one query — once as the
/// source of a link and once as its target — and the two have to be filtered
/// independently. A backlink from a page you cannot read must not appear at all;
/// a link *to* a page you cannot read is a different question, answered in
/// [`crate::index::graph`].
pub fn visible_as(alias: &str) -> String {
    VISIBLE.replace("pages.", &format!("{alias}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(raw: &str) -> Username {
        Username::parse(raw).expect("valid username")
    }

    /// Every branch of the fragment reads a column that exists, and the aliased
    /// spelling rewrites all of them rather than the first.
    #[test]
    fn the_aliased_form_rewrites_every_reference() {
        let aliased = visible_as("target");

        assert!(!aliased.contains("pages.visibility"));
        assert!(!aliased.contains("pages.owner"));
        assert!(aliased.contains("target.visibility"));
        assert!(aliased.contains("target.owner"));
        // `page_readers.slug = pages.slug` has to follow the alias too, or a
        // restricted page is matched against the wrong row.
        assert!(aliased.contains("page_readers.slug = target.slug"));
    }

    #[test]
    fn an_open_wiki_is_signed_in_and_an_anonymous_caller_is_not() {
        assert!(Audience::Everything.is_signed_in());
        assert!(Audience::Account(name("tim")).is_signed_in());
        assert!(!Audience::Public.is_signed_in());
    }
}
