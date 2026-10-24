//! The route registry: **the only place a route is registered** (IS-14, ENF-4).
//!
//! Every route the web surface serves goes through [`Routes::route`], which records it in a table with its
//! **authorization class**. `tests/route_enumeration.rs` reads that table and holds each entry to what it
//! declares:
//!
//! * [`Class::Authorized`]: the handler's data access goes through `authorize(_, action, _)`. The test proves it
//!   from behaviour, not from the declaration: an **anonymous** caller asking for a **private** repository must be
//!   answered `404` (existence-neutral), while the same request for a public repository whose store is unreadable
//!   is *not* a `404`. A handler that skipped the check would answer the stranger like the public one;
//! * [`Class::AuthorizedAny`]: the same, for a route that either of several actions opens (RFC 005 part 5);
//! * [`Class::Public`]: no repository data is involved; a **reason** is part of the entry, and an empty reason
//!   does not compile into a table (it panics at registration, so a test that builds the router fails).
//!
//! A route registered any other way (`Router::new().route(…)` outside this module) is not in the table, so the
//! same test scans the crate's source and fails on one. That is the mechanism's limit: it cannot see a router
//! assembled in another crate and merged in, nor a handler that is *declared* `Authorized` yet reaches data by a
//! second path that the probe does not exercise (the probe exercises the route's own path, once per route).
//!
//! The table is built for the transport routes and the SSH verbs to come: [`Class`] and [`Surface`] are
//! `#[non_exhaustive]`, an entry names its action, and the probe's expectation for an action other than `Read` is
//! "an anonymous caller is refused", which every later write handler must meet.

use axum::Router;
use axum::routing::MethodRouter;

pub use planeter_core::Action;

use crate::api::AppState;

/// Where a registered entry point lives. Only HTTP routes exist today; the SSH verbs (RFC 004 D-9) will be a
/// second surface in the same table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Surface {
    Http,
}

/// How a route is authorized (see the module documentation for how the test holds each to its word).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Class {
    /// The handler decides through `authorize(principal, action, resource, ctx)` before it touches repository
    /// data.
    Authorized(Action),
    /// Like [`Class::Authorized`], for a route that **either** of several actions opens (RFC 005 part 5, T5: the download of
    /// an accepted artifact is for whoever holds `review` or `merge-seal`). The handler decides through `authorize` for
    /// each and allows the request if any allows; the probe holds it to the same word as `Authorized`: an anonymous
    /// caller asking for a private repository is answered `404`. The table lists every action.
    AuthorizedAny(&'static [Action]),
    /// No repository data is served; the reason says why that is safe. Never empty.
    Public(&'static str),
}

impl Class {
    /// A public route, with its reason. Panics on an empty reason: an allowlist entry without one is a bug.
    pub fn public(reason: &'static str) -> Class {
        assert!(
            !reason.trim().is_empty(),
            "a public route needs a reason (IS-14)"
        );
        Class::Public(reason)
    }
}

/// One registered route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub surface: Surface,
    /// The path pattern, as axum writes it (`/{owner}/{name}/tree/{*path}`).
    pub path: &'static str,
    /// The HTTP methods the entry answers (upper case).
    pub methods: &'static [&'static str],
    pub class: Class,
}

/// The routes of one or more surfaces, with their table. Consumed by [`Routes::build`].
pub struct Routes {
    router: Router<AppState>,
    table: Vec<Entry>,
}

impl Default for Routes {
    fn default() -> Self {
        Self::new()
    }
}

impl Routes {
    pub fn new() -> Self {
        Routes {
            router: Router::new(),
            table: Vec::new(),
        }
    }

    /// Register one route. `methods` names the methods `handler` answers; the test checks that it is true.
    #[must_use]
    pub fn route(
        mut self,
        path: &'static str,
        class: Class,
        methods: &'static [&'static str],
        handler: MethodRouter<AppState>,
    ) -> Self {
        assert!(!methods.is_empty(), "a route answers at least one method");
        assert!(
            !self.table.iter().any(|e| e.path == path),
            "route {path} is registered twice"
        );
        self.router = self.router.route(path, handler);
        self.table.push(Entry {
            surface: Surface::Http,
            path,
            methods,
            class,
        });
        self
    }

    /// Add another set of routes (the API and the UI each build their own).
    #[must_use]
    pub fn merge(mut self, other: Routes) -> Self {
        for e in &other.table {
            assert!(
                !self.table.iter().any(|x| x.path == e.path),
                "route {} is registered twice",
                e.path
            );
        }
        self.router = self.router.merge(other.router);
        self.table.extend(other.table);
        self
    }

    /// The table, in registration order.
    pub fn table(&self) -> &[Entry] {
        &self.table
    }

    /// The router, with its state.
    pub fn build(self, state: AppState) -> Router {
        self.router.with_state(state)
    }
}
