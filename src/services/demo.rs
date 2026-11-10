//! Demo content for development: after setup, `cargo run` fills the site
//! with posts, pages, tags, a menu, comments and a message, written like a
//! personal site (dincertekin.com), so themes and screens can be tried with
//! something real-looking on them.
//!
//! Only in development builds: release builds leave this file out
//! completely, so real sites never get it. Tests leave it out of setup too
//! (`src/tests/demo.rs` checks it on its own).

use crate::db::comments::{self, NewComment};
use crate::db::posts::{self, ImportedPost};
use crate::db::{messages, settings, tags};

use sqlx::SqlitePool;

/// One post or page. `days_ago` dates it (negative: scheduled for later).
struct Demo {
    title: &'static str,
    slug: &'static str,
    days_ago: i64,
    status: &'static str,
    tags: &'static [&'static str],
    content: &'static str,
}

const PAGES: &[Demo] = &[
    Demo {
        title: "About",
        slug: "about",
        days_ago: 130,
        status: "published",
        tags: &[],
        content: ABOUT,
    },
    Demo {
        title: "Projects",
        slug: "projects",
        days_ago: 130,
        status: "published",
        tags: &[],
        content: PROJECTS,
    },
    Demo {
        title: "Now",
        slug: "now",
        days_ago: 3,
        status: "published",
        tags: &[],
        content: NOW,
    },
    Demo {
        title: "Contact",
        slug: "contact",
        days_ago: 130,
        status: "published",
        tags: &[],
        content: CONTACT,
    },
];

const POSTS: &[Demo] = &[
    Demo {
        title: "Why I'm building a lighter WordPress",
        slug: "why-a-lighter-wordpress",
        days_ago: 120,
        status: "published",
        tags: &["Bloogla", "Self-hosting"],
        content: WHY_BLOOGLA,
    },
    Demo {
        title: "Learning Rust by shipping something people depend on",
        slug: "learning-rust-by-shipping",
        days_ago: 96,
        status: "published",
        tags: &["Rust"],
        content: LEARNING_RUST,
    },
    Demo {
        title: "One program, no plugins",
        slug: "one-program-no-plugins",
        days_ago: 81,
        status: "published",
        tags: &["Bloogla"],
        content: ONE_PROGRAM,
    },
    Demo {
        title: "How a blog can update itself without trusting the download",
        slug: "signed-updates",
        days_ago: 63,
        status: "published",
        tags: &["Rust", "Security"],
        content: SIGNED_UPDATES,
    },
    Demo {
        title: "Five languages and one polite “you”",
        slug: "five-languages",
        days_ago: 47,
        status: "published",
        tags: &["Translation", "Bloogla"],
        content: FIVE_LANGUAGES,
    },
    Demo {
        title: "A blog that doesn't watch its readers",
        slug: "a-blog-that-doesnt-watch",
        days_ago: 31,
        status: "published",
        tags: &["Privacy"],
        content: PRIVACY,
    },
    Demo {
        title: "SQLite is enough",
        slug: "sqlite-is-enough",
        days_ago: 19,
        status: "published",
        tags: &["SQLite", "Rust"],
        content: SQLITE,
    },
    Demo {
        title: "Backups you'll have when you need them",
        slug: "backups-you-will-have",
        days_ago: 9,
        status: "published",
        tags: &["Self-hosting", "Bloogla"],
        content: BACKUPS,
    },
    Demo {
        title: "Bloogla 1.4: subscribers, storage and a Dashboard you arrange",
        slug: "bloogla-1-4",
        days_ago: 2,
        status: "published",
        tags: &["Bloogla"],
        content: RELEASE_1_4,
    },
    Demo {
        title: "Ideas for Bloogla 1.5",
        slug: "ideas-for-1-5",
        days_ago: 1,
        status: "draft",
        tags: &["Bloogla"],
        content: IDEAS,
    },
    Demo {
        title: "Signing S3 requests by hand",
        slug: "signing-s3-requests-by-hand",
        days_ago: -3,
        status: "scheduled",
        tags: &["Rust"],
        content: S3_SIGNING,
    },
];

/// A date `days_ago` days back (or ahead), at a believable hour, as stored.
fn date(days_ago: i64, hour: u32) -> String {
    let day = chrono::Utc::now().date_naive() - chrono::Duration::days(days_ago);
    format!(
        "{day} {hour:02}:{:02}:00",
        (days_ago.unsigned_abs() * 7) % 60
    )
}

/// Fill a freshly set-up site. Mistakes are logged, never fatal: it's only
/// a demo.
pub async fn add(pool: &SqlitePool, owner_id: i64) {
    if let Err(e) = add_everything(pool, owner_id).await {
        tracing::warn!("Couldn't add all the demo content: {e}");
    }
}

async fn add_everything(pool: &SqlitePool, owner_id: i64) -> Result<(), sqlx::Error> {
    let mut ids = std::collections::HashMap::new();
    for (demo, is_page) in PAGES
        .iter()
        .map(|d| (d, true))
        .chain(POSTS.iter().map(|d| (d, false)))
    {
        let id = posts::add_imported(
            pool,
            &ImportedPost {
                title: demo.title,
                slug: demo.slug,
                content: demo.content,
                cover_image: None,
                status: demo.status,
                published_at: &date(demo.days_ago, 9 + (demo.days_ago.unsigned_abs() % 9) as u32),
                is_page,
                author_id: owner_id,
            },
        )
        .await?;
        if !demo.tags.is_empty() {
            let names: Vec<String> = demo.tags.iter().map(|t| t.to_string()).collect();
            let tag_ids = tags::with_new(pool, &[], &names).await;
            tags::set_for_post(pool, id, &tag_ids).await;
        }
        ids.insert(demo.slug, id);
    }

    // A few readers' comments, and one waiting for approval.
    let comments_to_add: [(&str, &str, &str, &str, i64); 5] = [
        (
            "why-a-lighter-wordpress",
            "Elif",
            "I moved my recipe blog off WordPress last year for exactly this reason. Updating plugins every week was the only thing I did there.",
            "approved",
            118,
        ),
        (
            "why-a-lighter-wordpress",
            "Marius",
            "One file I can copy to a server is very appealing. How big is it?",
            "approved",
            117,
        ),
        (
            "learning-rust-by-shipping",
            "Jonas",
            "The part about reading the compiler errors slowly is so true. They're usually telling you exactly what to do.",
            "approved",
            94,
        ),
        (
            "a-blog-that-doesnt-watch",
            "Camille",
            "Thank you for the part about not loading fonts from other servers. Most people don't know that tells someone you visited.",
            "approved",
            29,
        ),
        (
            "backups-you-will-have",
            "Deniz",
            "Does the copy to storage include the pictures too, or only the database?",
            "pending",
            1,
        ),
    ];
    for (slug, name, text, status, days_ago) in comments_to_add {
        if let Some(post_id) = ids.get(slug) {
            comments::add(
                pool,
                &NewComment {
                    post_id: *post_id,
                    author_name: name,
                    author_email: "",
                    content: text,
                    status,
                    created_at: Some(date(days_ago, 18)),
                },
            )
            .await?;
        }
    }

    if let Some(contact) = ids.get("contact") {
        messages::add(
            pool,
            *contact,
            "Ayşe",
            "ayse@example.com",
            "Hi Dinçer, I run a small bookshop and would love a simple site for it. Is Bloogla a good fit for something that isn't really a blog?",
        )
        .await?;
    }

    settings::save(
        pool,
        &[
            (
                "blog_description",
                "Notes on building Bloogla, learning Rust and keeping the web small.".into(),
            ),
            (
                "nav_menu",
                "About | /about\nProjects | /projects\nNow | /now\nArchive | /archive\nContact | /contact"
                    .into(),
            ),
        ],
    )
    .await?;
    tracing::info!("Development build: added demo posts, pages and comments.");
    Ok(())
}

// ---- The texts ----

const ABOUT: &str = "Hi, I'm Dinçer.

I build [Bloogla](/projects), a small blog engine for people who want a website without looking after one. This is where I write about making it: what worked, what didn't, and what I learned along the way.

Most of what I write is about:

- **Building software for people who aren't developers.** If someone has to read a manual to publish a post, I've made a mistake.
- **Rust**, which I'm learning by using it for something real.
- **Keeping the web small**: pages that load fast, don't track anyone and still work in ten years.

If you'd like to say hello, the [contact page](/contact) reaches me directly.";

const PROJECTS: &str = "## Bloogla

A lighter WordPress in one small program. It runs the website, the admin panel and the HTTPS certificate, with no PHP, no database server and no plugins. Seven themes, five languages, signed updates and one-click backups.

[Source code on GitHub](https://github.com/dincertekin/bloogla)

## This site

Runs on Bloogla, of course. Whatever breaks here gets fixed there first.";

const NOW: &str = "*What I'm doing now, in the spirit of [nownownow.com](https://nownownow.com).*

- Finishing **Bloogla 1.4**: email subscriptions, copying backups to other storage, and a Dashboard you can arrange.
- Reading *The Rust Programming Language* again, this time the chapters I skipped.
- Writing less code and more tests before each release.";

const CONTACT: &str = "Questions about Bloogla, ideas, or just hello: write below and it lands in my inbox.

For bugs, a [GitHub issue](https://github.com/dincertekin/bloogla/issues) is even better, because others can find it too.

[contact-form]";

const WHY_BLOOGLA: &str = "Most people who ask me for a website don't want a website. They want to write something and have it appear on the internet, looking nice, without thinking about it again.

WordPress can do that, and it powers a huge part of the web for good reasons. But a small WordPress blog still comes with a database server, PHP, a list of plugins that each want updating, and a login page that bots knock on all day. The writing is the easy part; the upkeep is the work.

So I started building the thing I kept wishing existed:

1. **One program.** Copy it to a server, start it, open the link it prints.
2. **Everything in the browser.** Writing, themes, backups, updates. The command line is only for getting back in if you lose your password.
3. **Safe by default.** Two-factor login, strict security headers and updates that are checked before they're installed.

It's called Bloogla, and this site runs on it.

> The best maintenance is the maintenance you never have to do.

It isn't meant to replace WordPress for everyone. It's for the many sites that use a tenth of what WordPress offers and pay for all of it.";

const LEARNING_RUST: &str = "I started Bloogla knowing very little Rust. That was partly the point: I learn best when something real depends on the result.

A few things that helped:

- **Read the compiler errors slowly.** They're long because they're helpful. Most of them end with the exact change to make.
- **Keep the code boring.** Plain functions, plain structs, `match` instead of clever traits. When I come back to a file in three months, I want to understand it in one read.
- **Never `unwrap()` on anything a visitor sends.** A broken upload should get a friendly message, not take the site down.

Here's the kind of code I aim for. It isn't impressive, and that's what I like about it:

```rust
/// \"1 post\" or \"3 posts\", in the site's language.
pub fn count(lang: Lang, n: i64) -> String {
    let key = if n == 1 { \"{n} post\" } else { \"{n} posts\" };
    lang.t(key).replace(\"{n}\", &n.to_string())
}
```

The borrow checker and I still disagree sometimes. But every time it refused my code, it turned out to be right, usually about something that would have been a bug at three in the morning.";

const ONE_PROGRAM: &str = "Bloogla is a single file. That one decision shaped almost everything else.

**The admin panel is compiled in.** Its templates are checked when the program is built, so a typo in one is caught by me, not by you.

**Themes are loaded from a folder.** They're plain HTML and CSS, so anyone can make one without a compiler. The seven bundled themes are inside the program and unpacked on start.

**The database is a file too.** SQLite lives next to the program in `data/`, so a backup is a copy of one folder.

**There are no plugins.** This is the one people ask about most. Plugins are where most WordPress sites get broken into, and where most of their upkeep goes. Instead, the things nearly every blog needs are simply there: comments with spam protection, a contact form, an archive, RSS, a sitemap, analytics without cookies.

The result is boring in the best way: you start it, and it keeps running.";

const SIGNED_UPDATES: &str = "A program that updates itself is a program that downloads code from the internet and runs it. That deserves some care.

When Bloogla installs a new version, it doesn't trust the download just because it came from GitHub. Every release is **signed** with a key that never leaves my computer. Before replacing anything, Bloogla checks that signature against the public key built into it. A file that wasn't signed by that key is refused, wherever it came from.

The steps, roughly:

1. Ask GitHub for the latest release (only if you turned on checking).
2. Download the program and its signature.
3. Check the signature. If it doesn't match, stop and say so.
4. Keep the old program as `bloogla.old`, put the new one in its place and restart.

If anything goes wrong after that, renaming `bloogla.old` back undoes it.

It's a small amount of code for a lot of peace of mind, and it means a one-click update can't become someone else's one-click takeover.";

const FIVE_LANGUAGES: &str = "Bloogla speaks English, Deutsch, Français, Lietuvių and Türkçe. The admin panel, the themes and the emails all follow the site's language.

The rule I care about most is small: **every language uses the polite “you”**. *Sie* in German, *vous* in French, *jūs* in Lithuanian, *siz* in Turkish. Software talks to people who trust it with their writing, and it should sound warm but respectful, never like a chat with a stranger who's a bit too familiar.

A few things I learned:

- **Never build sentences from pieces.** “{n} posts” is one text, not a number glued to a word. Languages count differently; Lithuanian has more plural forms than English.
- **Write the English plainly first.** Short, clear sentences translate well. Clever ones don't.
- **Let the tests check.** If a text has no translation, `cargo test` lists it, so nothing ships half-translated.

Translations can be added without programming. If you'd like Bloogla in your language, I'd love the help.";

const PRIVACY: &str = "Visitors to this site aren't tracked. No cookies, no stored IP addresses, no analytics service, and no fonts or scripts from other servers.

That last one surprises people. Loading a font from a big font service tells that service that you visited this page. So do embedded share buttons and most analytics scripts. Bloogla's public pages load nothing from anywhere else.

I still like knowing which posts people read, so Bloogla counts **views per day** and **which sites sent readers**, and nothing about who they are. Bots, link previews and my own visits aren't counted.

It also keeps pages fast. A post here is mostly text, and text is small. If a post has $w$ words, its reading time is about

$$t = \\frac{w}{200}$$

minutes, which is how Bloogla works it out.

Privacy and speed turn out to want the same thing: send less.";

const SQLITE: &str = "People are sometimes surprised that Bloogla uses SQLite instead of a “real” database server. For a blog, SQLite *is* the real database.

- **It's a file.** No server to install, update or secure. Backups are a consistent copy made with `VACUUM INTO`, while the site keeps running.
- **It's fast.** Reading a post is a few lookups in a file the operating system already keeps in memory.
- **It's careful.** Bloogla turns on write-ahead logging, so readers never wait for writers.

The one rule I follow: **no query per item in a list.** A page of ten posts makes one query for the posts and one for their tags, not eleven.

```sql
SELECT id, title, slug FROM posts
WHERE status = 'published' AND trashed_at IS NULL
ORDER BY published_at DESC LIMIT 10;
```

A single-file database also makes moving easy. A Bloogla site is one folder: copy it to a new server, start the program, and you're done.";

const BACKUPS: &str = "Everyone agrees backups matter. Fewer people have one they could restore today.

Bloogla tries to make the good habit the default:

1. **Every day**, it saves a copy of the database on the server and keeps the last seven.
2. **Any time**, Settings → Backup gives you the whole site as one `.zip`: posts, pages, settings, people, pictures and videos.
3. **Restoring** shows what's inside a backup before it changes anything, and saves the site as it is first.

There's one thing a backup on the same server can't survive: losing the server. That's why Bloogla 1.4 can also copy the backup to storage you rent somewhere else, every day.

Whatever you use, try a restore once. A backup you've never restored is a hope, not a plan.";

const RELEASE_1_4: &str = "Bloogla 1.4 is out. The highlights:

- **Email subscriptions.** Readers can get new posts by email. They confirm with a link, and every email has an unsubscribe link.
- **Backups somewhere else.** Each day's backup can go to Backblaze B2, Cloudflare R2, Amazon S3 or any S3-compatible storage.
- **Site health.** The Dashboard tells you whether HTTPS, backups, email and updates are in order.
- **A Dashboard you arrange.** Drag the boxes into your own order and hide the ones you don't need.
- **A feed for each tag**, like [/tag/rust/rss.xml](/tag/rust/rss.xml).

Update from Settings → Updates. As always, download a backup first.";

const IDEAS: &str = "Not public yet, just notes.

- Disk space in Site health
- Exporting the subscriber list
- New screenshots for the README";

const S3_SIGNING: &str = "Bloogla copies backups to S3-compatible storage without an S3 library. Signing the requests by hand turned out to be about a hundred lines, and a good lesson in reading a specification carefully.

More soon.";
