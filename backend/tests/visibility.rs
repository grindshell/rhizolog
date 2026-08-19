//! Who can see which pages.
//!
//! The centre of this file is `nothing_about_a_private_page_reaches_anybody_else`,
//! which walks **every** endpoint that can return something about a page and
//! asserts that a private one appears in none of them. That test exists because
//! visibility is not one check in one handler: a page leaks through the listing,
//! through a search snippet, through a backlink's title, through the tag
//! histogram, through `most_linked`, and through the graph — six different
//! shapes, any one of which is enough.
//!
//! The rest of the file is the ladder itself, the write path, and the two
//! failure directions that matter: a typo must not publish a page, and narrowing
//! one must not make it unreadable by everybody including its author.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use rhizolog::{AppState, Assets, Index, Store, TimeStore, UserStore};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";

struct App {
    router: Router,
    _directory: TempDir,
}

struct Res {
    status: StatusCode,
    body: Value,
}

impl Res {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("<no code>")
    }
}

impl App {
    async fn new() -> Self {
        Self::with_anonymous_read(false).await
    }

    async fn with_anonymous_read(anonymous_read: bool) -> Self {
        let directory = TempDir::new().expect("temp dir");
        let store = Store::open(directory.path()).await.expect("open store");
        let times = TimeStore::open(directory.path())
            .await
            .expect("open time log");
        let users = UserStore::open(directory.path()).await.expect("open users");
        let index = Index::open(None).await.expect("open index");

        Self {
            router: rhizolog::router(AppState {
                store,
                times,
                users,
                index,
                usage: rhizolog::UsageTally::new(),
                assets: Assets::None,
                secure_cookies: false,
                anonymous_read,
            }),
            _directory: directory,
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> Res {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }

        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            None => builder.body(Body::empty()),
        }
        .expect("build request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router response");

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");

        Res {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }

    async fn get(&self, path: &str, token: &str) -> Res {
        self.send(Method::GET, path, None, Some(token)).await
    }

    async fn get_anonymously(&self, path: &str) -> Res {
        self.send(Method::GET, path, None, None).await
    }

    async fn post(&self, path: &str, body: Value, token: &str) -> Res {
        self.send(Method::POST, path, Some(body), Some(token)).await
    }

    async fn put(&self, path: &str, token: &str) -> Res {
        self.send(Method::PUT, path, None, Some(token)).await
    }

    /// Create an account and return a session token for it.
    async fn account(&self, username: &str, owner_token: Option<&str>) -> String {
        let created = self
            .send(
                Method::POST,
                "/api/users",
                Some(json!({
                    "username": username,
                    "password": PASSWORD,
                    "role": if owner_token.is_some() { "member" } else { "owner" },
                })),
                owner_token,
            )
            .await;
        assert_eq!(
            created.status,
            StatusCode::CREATED,
            "creating {username} failed: {:?}",
            created.body
        );

        let signed_in = self
            .send(
                Method::POST,
                "/api/auth/login",
                Some(json!({ "username": username, "password": PASSWORD })),
                None,
            )
            .await;
        assert_eq!(signed_in.status, StatusCode::OK, "{:?}", signed_in.body);
        signed_in.body["token"]
            .as_str()
            .expect("a token")
            .to_owned()
    }

    /// Write a page, asserting it worked.
    async fn write(&self, token: &str, body: Value) {
        let res = self.post("/api/pages", body.clone(), token).await;
        assert_eq!(
            res.status,
            StatusCode::CREATED,
            "writing {body} failed: {:?}",
            res.body
        );
    }
}

/// A wiki with two accounts. `tim` owns everything; `alice` is a member.
async fn two_accounts() -> (App, String, String) {
    let app = App::new().await;
    let tim = app.account("tim", None).await;
    let alice = app.account("alice", Some(&tim)).await;
    (app, tim, alice)
}

// ------------------------------------------------------------- the ladder

/// An existing wiki's pages carry no `visibility:` at all, and the moment
/// authentication goes on they have to stay readable by the people already
/// using it — without becoming readable by strangers.
#[tokio::test]
async fn an_unmarked_page_is_internal() {
    let (app, tim, alice) = two_accounts().await;
    app.write(
        &tim,
        json!({ "slug": "notes/plain", "content": "Ordinary.\n" }),
    )
    .await;

    let read = app.get("/api/pages/notes/plain", &alice).await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["visibility"], "internal");
}

#[tokio::test]
async fn each_rung_admits_exactly_who_it_says() {
    let (app, tim, alice) = two_accounts().await;

    app.write(
        &tim,
        json!({ "slug": "open", "content": "x", "visibility": "public" }),
    )
    .await;
    app.write(
        &tim,
        json!({ "slug": "shared", "content": "x", "visibility": "internal" }),
    )
    .await;
    app.write(
        &tim,
        json!({ "slug": "some", "content": "x", "visibility": "restricted", "readers": ["alice"] }),
    )
    .await;
    app.write(
        &tim,
        json!({ "slug": "mine", "content": "x", "visibility": "private" }),
    )
    .await;
    app.write(
        &tim,
        json!({ "slug": "theirs", "content": "x", "visibility": "restricted", "readers": ["bob"] }),
    )
    .await;

    // The owner reads everything of theirs, whatever it says.
    for slug in ["open", "shared", "some", "mine", "theirs"] {
        assert_eq!(
            app.get(&format!("/api/pages/{slug}"), &tim).await.status,
            StatusCode::OK,
            "the owner could not read {slug}"
        );
    }

    // Another account gets the three it is entitled to, and not the two it is
    // not.
    for (slug, expected) in [
        ("open", StatusCode::OK),
        ("shared", StatusCode::OK),
        ("some", StatusCode::OK),
        ("mine", StatusCode::NOT_FOUND),
        ("theirs", StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            app.get(&format!("/api/pages/{slug}"), &alice).await.status,
            expected,
            "wrong answer for {slug}"
        );
    }
}

/// **404, not 403.** A 403 confirms that something exists at a slug somebody
/// guessed, and for a private page the slug is usually the title.
#[tokio::test]
async fn a_page_you_cannot_read_is_indistinguishable_from_one_that_is_not_there() {
    let (app, tim, alice) = two_accounts().await;
    app.write(
        &tim,
        json!({ "slug": "secret/acquisition", "content": "x", "visibility": "private" }),
    )
    .await;

    let hidden = app.get("/api/pages/secret/acquisition", &alice).await;
    let absent = app.get("/api/pages/secret/nothing-here", &alice).await;

    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(hidden.code(), "page_not_found");
    assert_eq!(hidden.status, absent.status);
    assert_eq!(hidden.code(), absent.code());

    // The two responses differ only where they echo the slug that was asked
    // about, which is the caller's own input. Normalising it away is what makes
    // this an assertion about the *shape* of the answer rather than about two
    // strings that were never going to be equal.
    let shape = |res: &Res, slug: &str| res.body.to_string().replace(slug, "<slug>");
    assert_eq!(
        shape(&hidden, "secret/acquisition"),
        shape(&absent, "secret/nothing-here"),
        "a page that is hidden and a page that is absent answer differently"
    );
}

/// The one direction a typo in this field must never fail in.
#[tokio::test]
async fn an_unrecognised_visibility_hides_the_page_rather_than_publishing_it() {
    let (app, tim, alice) = two_accounts().await;
    let path = app_page_path(&app, "notes/typo");

    // Written by hand, because the API's own enum would refuse the word.
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("page directory");
    std::fs::write(&path, "---\nvisibility: privte\nowner: tim\n---\n\nOops.\n")
        .expect("write the page by hand");
    assert_eq!(
        app.post("/api/reindex", json!({}), &tim).await.status,
        StatusCode::OK
    );

    assert_eq!(
        app.get("/api/pages/notes/typo", &alice).await.status,
        StatusCode::NOT_FOUND,
        "a misspelled visibility fell back to something readable"
    );
    // Its owner can still get at it, which is what makes the mistake fixable.
    assert_eq!(
        app.get("/api/pages/notes/typo", &tim).await.status,
        StatusCode::OK
    );
}

fn app_page_path(app: &App, slug: &str) -> std::path::PathBuf {
    app._directory.path().join(format!("{slug}.md"))
}

// ------------------------------------------------------ the exhaustive check

/// The test this file exists for.
///
/// A private page is written, linked to from a page everybody can read, tagged,
/// and given a distinctive word in its body. Then every endpoint that can say
/// anything about a page is asked, and none of them may mention it — not its
/// slug, not its title, not its tag, and not the word in its body.
///
/// Each of those is a separate leak with a separate fix, which is why this is
/// one test over a list of endpoints rather than six tests that each remember to
/// check one thing.
#[tokio::test]
async fn nothing_about_a_private_page_reaches_anybody_else() {
    let (app, tim, alice) = two_accounts().await;

    app.write(
        &tim,
        json!({
            "slug": "secret/acquisition",
            "title": "Project Roadrunner",
            "tags": ["confidential"],
            "content": "The counterparty is Acme, valued at fourteen supercalifragilistic.\n",
            "visibility": "private",
        }),
    )
    .await;
    // An ordinary page that links to it, so the link graph has a way to reach it.
    app.write(
        &tim,
        json!({
            "slug": "notes/plans",
            "content": "See [[secret/acquisition]] for the details.\n",
        }),
    )
    .await;

    let forbidden = [
        "secret/acquisition",
        "Project Roadrunner",
        "confidential",
        "supercalifragilistic",
        "Acme",
    ];

    for path in [
        "/api/pages",
        "/api/pages?limit=500",
        "/api/pages?tag=confidential",
        "/api/pages?prefix=secret",
        "/api/pages?segment=secret",
        "/api/search?q=supercalifragilistic",
        "/api/search?q=Acme",
        "/api/search?q=Roadrunner",
        "/api/tags",
        "/api/stats",
        "/api/graph",
        "/api/graph?wanted=true",
        "/api/graph?root=notes/plans&depth=3",
        "/api/links/notes/plans",
        "/api/health",
    ] {
        let res = app.get(path, &alice).await;
        assert_eq!(res.status, StatusCode::OK, "{path} failed: {:?}", res.body);

        let rendered = res.body.to_string();
        for secret in forbidden {
            assert!(
                !rendered.contains(secret),
                "{path} leaked {secret:?}:\n{rendered}"
            );
        }
    }

    // `/api/links/{slug}` is asked about separately, because it echoes the slug
    // the caller supplied and so cannot be held to the sweep above. What it must
    // not do is confirm that anything is there.
    let asked_directly = app.get("/api/links/secret/acquisition", &alice).await;
    assert_eq!(asked_directly.status, StatusCode::OK);
    assert_eq!(
        asked_directly.body["exists"], false,
        "the links endpoint confirmed that a private page exists"
    );
    for secret in ["Project Roadrunner", "confidential", "supercalifragilistic"] {
        assert!(
            !asked_directly.body.to_string().contains(secret),
            "the links endpoint leaked {secret:?}"
        );
    }
}

/// The limit of what any of this can promise, stated as a test so nobody
/// mistakes it for a bug later.
///
/// A wikilink is written in a page's body. Anyone who can read that body can
/// read the slug in it, and no amount of filtering in the index changes that —
/// the markdown is the markdown. What visibility protects is the private page's
/// *contents, title and existence*, not the fact that somebody once typed its
/// slug somewhere else.
///
/// This is also why a link to a page you cannot read is **dropped** from the
/// graph rather than shown as wanted: `wanted` is a positive claim that nobody
/// has written the page, and that claim would be false.
#[tokio::test]
async fn a_slug_written_in_a_readable_body_is_readable_and_that_is_accepted() {
    let (app, tim, alice) = two_accounts().await;

    app.write(
        &tim,
        json!({
            "slug": "secret/acquisition",
            "title": "Project Roadrunner",
            "content": "The counterparty is Acme.\n",
            "visibility": "private",
        }),
    )
    .await;
    app.write(
        &tim,
        json!({ "slug": "notes/plans", "content": "See [[secret/acquisition]].\n" }),
    )
    .await;

    let readable = app.get("/api/pages/notes/plans", &alice).await;
    assert_eq!(readable.status, StatusCode::OK);
    assert!(
        readable.body["content"]
            .as_str()
            .expect("content")
            .contains("secret/acquisition"),
        "the body is served as written, which is the point"
    );

    // What is protected is everything behind the slug.
    assert_eq!(
        app.get("/api/pages/secret/acquisition", &alice)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert!(
        !app.get("/api/search?q=Acme", &alice)
            .await
            .body
            .to_string()
            .contains("Acme")
    );
}

/// The subtlest of the six, and the one that fails in the wrong direction if the
/// filter is written as "the target is visible" rather than "the target is not
/// an invisible page".
///
/// A link to a page you cannot read must vanish. If it merely lost its title it
/// would be drawn as a **wanted** page — named by its slug, and advertised as
/// something worth writing.
#[tokio::test]
async fn a_link_to_a_private_page_is_not_reported_as_a_page_worth_writing() {
    let (app, tim, alice) = two_accounts().await;

    app.write(
        &tim,
        json!({ "slug": "secret/acquisition", "content": "x", "visibility": "private" }),
    )
    .await;
    app.write(
        &tim,
        json!({
            "slug": "notes/plans",
            "content": "See [[secret/acquisition]] and [[notes/genuinely-unwritten]].\n",
        }),
    )
    .await;

    let stats = app.get("/api/stats", &alice).await;
    let wanted: Vec<&str> = stats.body["wanted"]
        .as_array()
        .expect("wanted")
        .iter()
        .map(|page| page["slug"].as_str().expect("a slug"))
        .collect();

    // The genuinely unwritten one is still there — that is the feature.
    assert_eq!(wanted, ["notes/genuinely-unwritten"]);
    assert_eq!(stats.body["wanted_count"], 1);

    // And the outbound link is gone from the page that made it, rather than
    // showing as unresolved.
    let links = app.get("/api/links/notes/plans", &alice).await;
    let targets: Vec<&str> = links.body["outbound"]
        .as_array()
        .expect("outbound")
        .iter()
        .map(|link| link["target"].as_str().expect("a target"))
        .collect();
    assert_eq!(targets, ["notes/genuinely-unwritten"]);
}

/// A backlink carries the linking page's slug *and* its title, so an unfiltered
/// inbound list is a directory of the private pages that happen to link here.
#[tokio::test]
async fn a_backlink_from_a_private_page_does_not_appear() {
    let (app, tim, alice) = two_accounts().await;

    app.write(
        &tim,
        json!({ "slug": "notes/target", "content": "Target.\n" }),
    )
    .await;
    app.write(
        &tim,
        json!({
            "slug": "secret/plans",
            "title": "Project Roadrunner",
            "content": "See [[notes/target]].\n",
            "visibility": "private",
        }),
    )
    .await;
    app.write(
        &tim,
        json!({ "slug": "notes/public-referrer", "content": "See [[notes/target]].\n" }),
    )
    .await;

    let links = app.get("/api/links/notes/target", &alice).await;
    let inbound: Vec<&str> = links.body["inbound"]
        .as_array()
        .expect("inbound")
        .iter()
        .map(|link| link["slug"].as_str().expect("a slug"))
        .collect();

    assert_eq!(inbound, ["notes/public-referrer"]);

    // Its owner sees both, which is what says the link is really there.
    let owners_view = app.get("/api/links/notes/target", &tim).await;
    assert_eq!(
        owners_view.body["inbound"]
            .as_array()
            .expect("inbound")
            .len(),
        2
    );
}

/// A count is a small leak and a tag nobody else uses is a large one.
#[tokio::test]
async fn a_tag_used_only_by_private_pages_does_not_exist_for_anybody_else() {
    let (app, tim, alice) = two_accounts().await;

    app.write(
        &tim,
        json!({ "slug": "secret/a", "tags": ["acquisition"], "content": "x", "visibility": "private" }),
    )
    .await;
    app.write(
        &tim,
        json!({ "slug": "notes/a", "tags": ["rust", "acquisition"], "content": "x" }),
    )
    .await;

    let tags = app.get("/api/tags", &alice).await;
    let counts: Vec<(String, u64)> = tags.body["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .map(|tag| {
            (
                tag["tag"].as_str().expect("a tag").to_owned(),
                tag["pages"].as_u64().expect("a count"),
            )
        })
        .collect();

    // `acquisition` survives because a page they *can* read carries it — but
    // counted once, not twice.
    assert!(
        counts.contains(&("acquisition".to_owned(), 1)),
        "{counts:?}"
    );
    assert!(counts.contains(&("rust".to_owned(), 1)), "{counts:?}");

    // The owner sees the real count.
    let owners = app.get("/api/tags", &tim).await;
    let owners_counts: Vec<u64> = owners.body["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .filter(|tag| tag["tag"] == "acquisition")
        .map(|tag| tag["pages"].as_u64().expect("a count"))
        .collect();
    assert_eq!(owners_counts, [2]);
}

/// A paginated total that counts pages the caller cannot read is both a wrong
/// number and a statement about what exists.
#[tokio::test]
async fn totals_count_only_what_the_caller_can_see() {
    let (app, tim, alice) = two_accounts().await;

    for n in 0..3 {
        app.write(
            &tim,
            json!({ "slug": format!("secret/{n}"), "content": "x", "visibility": "private" }),
        )
        .await;
    }
    app.write(&tim, json!({ "slug": "notes/open", "content": "x" }))
        .await;

    let listing = app.get("/api/pages", &alice).await;
    assert_eq!(listing.body["total"], 1);
    assert_eq!(listing.body["pages"].as_array().expect("pages").len(), 1);

    assert_eq!(app.get("/api/health", &alice).await.body["pages"], 1);
    assert_eq!(app.get("/api/stats", &alice).await.body["pages"], 1);

    // Four, for the account that owns them.
    assert_eq!(app.get("/api/pages", &tim).await.body["total"], 4);
    assert_eq!(app.get("/api/health", &tim).await.body["pages"], 4);
}

// --------------------------------------------------------------- the writes

/// A private page whose owner is nobody is readable by nobody, including its
/// author — correct, useless, and only fixable from the server's filesystem.
#[tokio::test]
async fn creating_a_private_page_makes_you_its_owner() {
    let (app, tim, _alice) = two_accounts().await;

    app.write(
        &tim,
        json!({ "slug": "mine", "content": "x", "visibility": "private" }),
    )
    .await;

    let read = app.get("/api/pages/mine", &tim).await;
    assert_eq!(read.body["owner"], "tim");
    assert_eq!(read.body["visibility"], "private");
}

/// The same guard from the other direction: narrowing a page and clearing its
/// owner in one request is how you lose one.
#[tokio::test]
async fn a_page_cannot_be_narrowed_into_being_unreadable() {
    let (app, tim, _alice) = two_accounts().await;
    app.write(&tim, json!({ "slug": "notes/a", "content": "x" }))
        .await;

    let res = app
        .send(
            Method::PATCH,
            "/api/pages/notes/a",
            Some(json!({ "visibility": "private", "owner": null })),
            Some(&tim),
        )
        .await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "ownerless_page");
    assert_eq!(res.body["error"]["details"]["visibility"], "private");

    // And the page is untouched.
    assert_eq!(
        app.get("/api/pages/notes/a", &tim).await.body["visibility"],
        "internal"
    );
}

/// Every write path, not just the read one. `PUT` on a page you cannot see
/// would otherwise be an oracle for whether a slug is taken by something
/// private — and would overwrite it.
#[tokio::test]
async fn a_page_you_cannot_read_is_a_page_you_cannot_change() {
    let (app, tim, alice) = two_accounts().await;
    app.write(
        &tim,
        json!({ "slug": "secret/plans", "content": "Original.\n", "visibility": "private" }),
    )
    .await;

    for (method, path, body) in [
        (
            Method::PUT,
            "/api/pages/secret/plans",
            Some(json!({ "content": "Overwritten.\n" })),
        ),
        (
            Method::PATCH,
            "/api/pages/secret/plans",
            Some(json!({ "content": "Overwritten.\n" })),
        ),
        (Method::DELETE, "/api/pages/secret/plans", None),
        (
            Method::POST,
            "/api/move",
            Some(json!({ "from": "secret/plans", "to": "mine/now" })),
        ),
    ] {
        let res = app.send(method.clone(), path, body, Some(&alice)).await;
        assert_eq!(
            res.status,
            StatusCode::NOT_FOUND,
            "{method} {path} was allowed"
        );
    }

    // Still there, still saying what it said.
    let read = app.get("/api/pages/secret/plans", &tim).await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["content"], "Original.\n");
}

/// The reader list is what `restricted` is for, and taking somebody off it has
/// to take effect on the next request rather than at the next reindex.
#[tokio::test]
async fn removing_a_reader_takes_effect_immediately() {
    let (app, tim, alice) = two_accounts().await;
    app.write(
        &tim,
        json!({ "slug": "some", "content": "x", "visibility": "restricted", "readers": ["alice"] }),
    )
    .await;

    assert_eq!(
        app.get("/api/pages/some", &alice).await.status,
        StatusCode::OK
    );

    let patched = app
        .send(
            Method::PATCH,
            "/api/pages/some",
            Some(json!({ "readers": [] })),
            Some(&tim),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK);

    assert_eq!(
        app.get("/api/pages/some", &alice).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.get("/api/pages", &alice).await.body["total"], 0);
}

/// An owner administers accounts. That is not a licence to read somebody's
/// private pages, and the line is deliberate — see `knowledge-base/visibility.md`.
#[tokio::test]
async fn an_owner_is_not_a_superuser_over_content() {
    let (app, tim, alice) = two_accounts().await;

    app.write(
        &alice,
        json!({ "slug": "alice/diary", "content": "x", "visibility": "private" }),
    )
    .await;

    assert_eq!(
        app.get("/api/pages/alice/diary", &tim).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.get("/api/pages", &tim).await.body["total"], 0);
}

// ----------------------------------------------------------- anonymous read

/// Two deliberate acts, and this test is the first one on its own.
#[tokio::test]
async fn a_public_page_is_not_public_until_the_instance_says_so() {
    let app = App::new().await;
    let tim = app.account("tim", None).await;
    app.write(
        &tim,
        json!({ "slug": "open", "content": "Anyone.\n", "visibility": "public" }),
    )
    .await;

    assert_eq!(
        app.get_anonymously("/api/pages/open").await.status,
        StatusCode::UNAUTHORIZED,
        "a public page was served without RHIZOLOG_ANONYMOUS_READ"
    );
    assert_eq!(
        app.get_anonymously("/api/pages").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn with_anonymous_read_a_public_page_is_readable_and_nothing_else_is() {
    let app = App::with_anonymous_read(true).await;
    let tim = app.account("tim", None).await;

    app.write(
        &tim,
        json!({ "slug": "open", "title": "Open", "content": "Anyone may read this.\n", "visibility": "public" }),
    )
    .await;
    app.write(
        &tim,
        json!({ "slug": "shared", "title": "Shared", "content": "Accounts only.\n" }),
    )
    .await;

    let read = app.get_anonymously("/api/pages/open").await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["content"], "Anyone may read this.\n");

    assert_eq!(
        app.get_anonymously("/api/pages/shared").await.status,
        StatusCode::NOT_FOUND
    );

    let listing = app.get_anonymously("/api/pages").await;
    assert_eq!(listing.body["total"], 1);

    let search = app.get_anonymously("/api/search?q=Accounts").await;
    assert_eq!(search.body["total"], 0, "an internal page was searchable");
}

/// Anonymous read is a read. There is no configuration in Rhizolog that lets an
/// unauthenticated caller change anything.
#[tokio::test]
async fn anonymous_read_grants_no_writes_and_no_side_channels() {
    let app = App::with_anonymous_read(true).await;
    let tim = app.account("tim", None).await;
    app.write(
        &tim,
        json!({ "slug": "open", "content": "x", "visibility": "public" }),
    )
    .await;

    for (method, path, body) in [
        (
            Method::POST,
            "/api/pages",
            Some(json!({ "slug": "sneaky", "content": "x" })),
        ),
        (
            Method::PUT,
            "/api/pages/open",
            Some(json!({ "content": "defaced" })),
        ),
        (Method::DELETE, "/api/pages/open", None),
        (Method::POST, "/api/reindex", None),
        (
            Method::POST,
            "/api/render",
            Some(json!({ "content": "# hello" })),
        ),
        // Not page content: what the operator was doing, and when.
        (Method::GET, "/api/times", None),
        (Method::GET, "/api/pins", None),
        (Method::GET, "/api/time-stats", None),
        (Method::GET, "/api/users", None),
    ] {
        let res = app.send(method.clone(), path, body, None).await;
        assert_eq!(
            res.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} was allowed anonymously"
        );
    }

    // And the route-usage telemetry is not handed out with the public pages.
    let stats = app.get_anonymously("/api/stats").await;
    assert_eq!(stats.status, StatusCode::OK);
    assert_eq!(
        stats.body["api_usage"].as_array().expect("api_usage").len(),
        0
    );
}

// ------------------------------------------------------------- consistency

// ------------------------------------------------- the wiki-wide side channels

/// Pins and the time log are shared by everyone who can sign in, and both label
/// a page by joining `pages` at read time. That join is where a private page's
/// **title** would otherwise walk out — so it carries the predicate, and a pin
/// to a page you may not read is indistinguishable from a pin to one that has
/// been deleted.
#[tokio::test]
async fn a_pin_to_a_page_you_cannot_read_carries_no_title() {
    let (app, tim, alice) = two_accounts().await;
    app.write(
        &tim,
        json!({
            "slug": "secret/acquisition",
            "title": "Project Roadrunner",
            "content": "The counterparty is Acme.\n",
            "visibility": "private",
        }),
    )
    .await;

    let pinned = app.put("/api/pins/secret/acquisition", &tim).await;
    assert_eq!(pinned.status, StatusCode::OK, "{:?}", pinned.body);
    assert_eq!(pinned.body["title"], "Project Roadrunner");
    assert_eq!(pinned.body["exists"], true);

    let theirs = app.get("/api/pins", &alice).await;
    assert_eq!(theirs.status, StatusCode::OK);
    let pins = theirs.body["pins"].as_array().expect("pins");

    // The pin itself stays. It is wiki-wide state, and the slug is what a pin
    // *is* — hiding the row would be hiding somebody else's menu entry, not
    // protecting the page. The title is the page's, and that is withheld.
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0]["slug"], "secret/acquisition");
    assert_eq!(pins[0]["exists"], false);
    assert_eq!(pins[0]["title"], "secret/acquisition");
    assert!(
        !theirs.body.to_string().contains("Project Roadrunner"),
        "the pin list leaked the title of a page alice cannot read"
    );
}

/// `PUT /api/pins/{slug}` answers `404` for a page that is not there, so it must
/// answer `404` for a page you may not read — or it becomes the existence oracle
/// that reading the page itself was careful not to be.
#[tokio::test]
async fn a_page_you_cannot_read_cannot_be_pinned_either() {
    let (app, tim, alice) = two_accounts().await;
    app.write(
        &tim,
        json!({ "slug": "secret/acquisition", "content": "x", "visibility": "private" }),
    )
    .await;

    let hidden = app.put("/api/pins/secret/acquisition", &alice).await;
    let absent = app.put("/api/pins/secret/nothing-here", &alice).await;

    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{:?}", hidden.body);
    assert_eq!(hidden.status, absent.status);
    assert_eq!(hidden.code(), absent.code());
    // Nothing was pinned, so nothing new appears in the shared menu either.
    let pins = app.get("/api/pins", &tim).await;
    assert!(pins.body["pins"].as_array().expect("pins").is_empty());
}

/// The same join, in the time log. What is *not* withheld is the slug: an entry
/// says which pages the time was spent on, that list is the entry's own content,
/// and the log is readable by every account. This is the same limit
/// `a_slug_written_in_a_readable_body_is_readable_and_that_is_accepted` states
/// for page bodies.
#[tokio::test]
async fn a_time_entry_against_a_page_you_cannot_read_names_no_title() {
    let (app, tim, alice) = two_accounts().await;
    app.write(
        &tim,
        json!({
            "slug": "secret/acquisition",
            "title": "Project Roadrunner",
            "content": "The counterparty is Acme.\n",
            "visibility": "private",
        }),
    )
    .await;

    let logged = app
        .post(
            "/api/times",
            json!({
                "name": "Deep work",
                "start": "2026-08-18T09:00:00Z",
                "end": "2026-08-18T11:00:00Z",
                "pages": ["secret/acquisition"],
            }),
            &tim,
        )
        .await;
    assert_eq!(logged.status, StatusCode::CREATED, "{:?}", logged.body);
    assert_eq!(logged.body["pages"][0]["title"], "Project Roadrunner");
    let id = logged.body["id"].as_str().expect("an id").to_owned();

    let stats = "/api/time-stats?at=2026-08-18T12:00:00Z";
    for path in ["/api/times", &format!("/api/times/{id}"), stats] {
        let res = app.get(path, &alice).await;
        assert_eq!(res.status, StatusCode::OK, "{path} failed: {:?}", res.body);
        assert!(
            !res.body.to_string().contains("Project Roadrunner"),
            "{path} leaked the title of a page alice cannot read:\n{}",
            res.body
        );
    }

    let listed = app.get("/api/times", &alice).await;
    let page = &listed.body["times"][0]["pages"][0];
    assert_eq!(page["slug"], "secret/acquisition");
    assert_eq!(page["title"], Value::Null);
    assert_eq!(page["exists"], false);

    // Ranked in the statistics under its slug, which is the label an unwritten
    // page already gets — not dropped, because the time really was spent.
    let ranked = app.get(stats, &alice).await;
    assert!(
        ranked.body.to_string().contains("secret/acquisition"),
        "the entry vanished from the statistics instead of losing its title"
    );

    // And nothing changed for tim, who can read the page.
    let his = app.get("/api/times", &tim).await;
    assert_eq!(
        his.body["times"][0]["pages"][0]["title"],
        "Project Roadrunner"
    );
}

/// Visibility is decided twice: in SQL for the listing, and in Rust against a
/// file that has just been read. Two spellings of one rule is exactly the kind
/// of duplication that drifts, so this asserts they agree — for every rung, for
/// three different viewers.
#[tokio::test]
async fn the_single_page_read_and_the_listing_agree_about_every_page() {
    let (app, tim, alice) = two_accounts().await;

    let pages = [
        json!({ "slug": "a-public", "content": "x", "visibility": "public" }),
        json!({ "slug": "b-internal", "content": "x", "visibility": "internal" }),
        json!({ "slug": "c-unmarked", "content": "x" }),
        json!({ "slug": "d-restricted-alice", "content": "x", "visibility": "restricted", "readers": ["alice"] }),
        json!({ "slug": "e-restricted-nobody", "content": "x", "visibility": "restricted" }),
        json!({ "slug": "f-private", "content": "x", "visibility": "private" }),
    ];
    for page in &pages {
        app.write(&tim, page.clone()).await;
    }

    for token in [&tim, &alice] {
        let listing = app.get("/api/pages?limit=500", token).await;
        let listed: Vec<String> = listing.body["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .map(|page| page["slug"].as_str().expect("a slug").to_owned())
            .collect();

        for page in &pages {
            let slug = page["slug"].as_str().expect("a slug");
            let readable_directly =
                app.get(&format!("/api/pages/{slug}"), token).await.status == StatusCode::OK;

            assert_eq!(
                readable_directly,
                listed.contains(&slug.to_owned()),
                "{slug}: reading it directly and finding it in the listing disagree"
            );
        }
    }
}
