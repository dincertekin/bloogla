//! Interface languages for the admin panel, the theme and emails.
//!
//! English text is the key: code and templates write `me.t("Posts")`, and each
//! language file maps that English text to its own. Anything a language
//! doesn't translate is shown in English.
//!
//! Each language is one file in this folder describing everything about it:
//! its name, how dates look, its plural rule and its translations.
//!
//! To add a language:
//! 1. Copy `de.rs` to `<code>.rs` (e.g. `nl.rs`) and translate the right-hand side.
//! 2. Add `mod <code>;` and `&<code>::LANGUAGE` to [`LANGUAGES`] below.
//!
//! `cargo test` checks that every language translates every piece of text and
//! keeps its `{placeholders}`.

mod de;
mod en;
mod es;
mod fr;
mod it;
mod ja;
mod pt;
mod ru;
mod tr;
mod zh;

use std::collections::HashMap;
use std::sync::OnceLock;

/// Everything Bloogla needs to know about one language.
pub struct Language {
    /// Code as used in URLs, settings and `<html lang>`, e.g. `de`.
    pub code: &'static str,
    /// The language's own name, for language pickers, e.g. `Deutsch`.
    pub name: &'static str,
    /// Short month names, January first, as used in dates.
    pub months: [&'static str; 12],
    /// Full date: `{d}` day, `{dd}` two-digit day, `{m}` month name, `{y}` year.
    pub date: &'static str,
    /// Day and month only, same placeholders.
    pub day_month: &'static str,
    /// Whether a count takes the singular form ("1 post" vs "2 posts").
    pub is_singular: fn(i64) -> bool,
    /// `(English, translation)` pairs. Empty for English itself.
    pub translations: &'static [(&'static str, &'static str)],
}

/// Every language Bloogla speaks, in the order language pickers show them.
/// The first one is the default.
pub const LANGUAGES: &[&Language] = &[
    &en::LANGUAGE,
    &de::LANGUAGE,
    &es::LANGUAGE,
    &fr::LANGUAGE,
    &it::LANGUAGE,
    &pt::LANGUAGE,
    &tr::LANGUAGE,
    &ru::LANGUAGE,
    &ja::LANGUAGE,
    &zh::LANGUAGE,
];

/// Plural rule for languages where only exactly one is singular (English, German...).
pub fn singular_if_one(n: i64) -> bool {
    n == 1
}

/// Plural rule for languages where zero is singular too (French, Brazilian Portuguese).
pub fn singular_if_zero_or_one(n: i64) -> bool {
    n == 0 || n == 1
}

/// A language, as stored on people and in settings. Cheap to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lang(usize);

impl Lang {
    /// Every language, for pickers.
    pub fn all() -> Vec<Lang> {
        (0..LANGUAGES.len()).map(Lang).collect()
    }

    /// The language with this code (`de`), if Bloogla has it.
    pub fn parse(code: &str) -> Option<Self> {
        LANGUAGES.iter().position(|l| l.code == code).map(Lang)
    }

    fn info(self) -> &'static Language {
        LANGUAGES[self.0]
    }

    pub fn code(self) -> &'static str {
        self.info().code
    }

    /// The language's own name, for language pickers.
    pub fn native_name(self) -> &'static str {
        self.info().name
    }

    /// Pick a language from an `Accept-Language` header (English if none match).
    pub fn from_accept_language(header: &str) -> Self {
        header
            .split(',')
            .filter_map(|part| part.split(';').next())
            .filter_map(|tag| Lang::parse(&tag.trim().split('-').next()?.to_lowercase()))
            .next()
            .unwrap_or_default()
    }

    fn translations(self) -> &'static HashMap<&'static str, &'static str> {
        static MAPS: OnceLock<Vec<HashMap<&'static str, &'static str>>> = OnceLock::new();
        let maps = MAPS.get_or_init(|| {
            LANGUAGES
                .iter()
                .map(|l| l.translations.iter().copied().collect())
                .collect()
        });
        &maps[self.0]
    }

    /// Translate a piece of interface text.
    pub fn t(self, key: &'static str) -> &'static str {
        self.translations().get(key).copied().unwrap_or(key)
    }

    /// Translate text that isn't known at compile time (e.g. an error message
    /// returned by another function); unknown text is returned unchanged.
    pub fn t_owned(self, key: &str) -> String {
        self.translations()
            .get(key)
            .copied()
            .unwrap_or(key)
            .to_string()
    }

    /// Translate and fill placeholders. `values` follow the order of the
    /// `{names}` in the English key; translations may use them in any order.
    pub fn fill(self, key: &'static str, values: &[&dyn std::fmt::Display]) -> String {
        let mut text = self.t(key).to_string();
        for (name, value) in placeholder_names(key).into_iter().zip(values) {
            text = text.replace(&format!("{{{name}}}"), &value.to_string());
        }
        text
    }

    /// One placeholder: `tv("Since {date}", date)`.
    pub fn tv(self, key: &'static str, value: impl std::fmt::Display) -> String {
        self.fill(key, &[&value])
    }

    /// Two placeholders, filled in order.
    pub fn tv2(
        self,
        key: &'static str,
        a: impl std::fmt::Display,
        b: impl std::fmt::Display,
    ) -> String {
        self.fill(key, &[&a, &b])
    }

    /// Three placeholders, filled in order.
    pub fn tv3(
        self,
        key: &'static str,
        a: impl std::fmt::Display,
        b: impl std::fmt::Display,
        c: impl std::fmt::Display,
    ) -> String {
        self.fill(key, &[&a, &b, &c])
    }

    /// "1 post" / "3 posts": `one` and `many` are English texts containing
    /// `{n}`; the language's plural rule picks which translation to use.
    pub fn count(self, n: i64, one: &'static str, many: &'static str) -> String {
        let key = if (self.info().is_singular)(n) {
            one
        } else {
            many
        };
        self.t(key).replace("{n}", &n.to_string())
    }

    fn format_date(self, pattern: &str, date: chrono::NaiveDate) -> String {
        use chrono::Datelike;
        let month = self.info().months[date.month0() as usize];
        pattern
            .replace("{dd}", &format!("{:02}", date.day()))
            .replace("{d}", &date.day().to_string())
            .replace("{m}", month)
            .replace("{y}", &date.year().to_string())
    }

    /// A full date: `Oct 2, 2026`, `2. Okt. 2026`, `2026年10月2日`...
    pub fn date(self, date: chrono::NaiveDate) -> String {
        self.format_date(self.info().date, date)
    }

    /// Day and month: `Oct 2`, `2. Okt.`...
    pub fn day_month(self, date: chrono::NaiveDate) -> String {
        self.format_date(self.info().day_month, date)
    }

    /// Theme strings for Tera, keyed by id: `{{ t.back_to_all_posts }}`.
    pub fn theme_strings(self) -> HashMap<&'static str, &'static str> {
        THEME_STRINGS
            .iter()
            .map(|(id, english)| (*id, self.t(english)))
            .collect()
    }
}

/// `{name}` placeholders in order of appearance.
fn placeholder_names(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        names.push(&rest[start + 1..start + len]);
        rest = &rest[start + len + 1..];
    }
    names
}

/// Strings the bundled themes use as `{{ t.<id> }}`: (id, English text).
/// Ids avoid Tera's dot-path lookup, which breaks on keys containing periods.
pub const THEME_STRINGS: &[(&str, &str)] = &[
    ("all", "All"),
    ("all_projects", "All projects"),
    ("all_tags", "All Tags"),
    ("all_videos", "All videos"),
    ("back_to_all_posts", "Back to all posts"),
    ("by_name", "by {name}"),
    ("comment", "Comment"),
    ("comments", "Comments"),
    ("contents", "Contents"),
    ("copied", "Copied"),
    ("copy", "Copy"),
    ("copy_failed", "Copy failed"),
    ("email", "Email"),
    ("get_new_posts_by_email", "Get new posts by email"),
    ("latest_updates", "Latest updates"),
    ("latest_videos", "Latest videos"),
    ("leave_a_comment", "Leave a comment"),
    ("mentioned_this", "mentioned this"),
    ("more_videos", "More videos"),
    ("n1_comment", "1 comment"),
    ("n1_view", "1 view"),
    ("n_comments", "{n} comments"),
    ("n_min_read", "{n} min read"),
    ("n_views", "{n} views"),
    ("name", "Name"),
    ("never_shown", "Never shown"),
    ("newer", "Newer"),
    ("next", "Next"),
    ("no_posts_found", "No posts found."),
    (
        "no_posts_found_with_tag_tag",
        "No posts found with tag “#{tag}”.",
    ),
    (
        "no_spam_unsubscribe_any_time",
        "No spam. Unsubscribe any time.",
    ),
    ("older", "Older"),
    ("optional", "(optional)"),
    ("page_current_of_total", "Page {current} of {total}"),
    ("page_not_found", "Page not found"),
    ("pagination", "Pagination"),
    ("post_comment", "Post comment"),
    ("posts_tagged_with_hashtag", "Posts tagged with “#{tag}”"),
    ("posts_tagged_with_tag", "Posts tagged with {tag}"),
    ("powered_by_name", "Powered by {name}"),
    ("previous", "Previous"),
    ("search_docs", "Search the docs..."),
    ("search_posts", "Search posts..."),
    ("search_query", "Search: {query}"),
    ("search_results_for_query", "Search results for “{query}”"),
    ("search_videos", "Search videos..."),
    ("skip_to_content", "Skip to content"),
    ("subscribe", "Subscribe"),
    ("tag_tag", "Tag: {tag}"),
    ("tags", "Tags"),
    (
        "the_page_you_are_looking_for",
        "The page you are looking for doesn't exist or has moved.",
    ),
    ("website", "Website"),
    ("your_email", "Your email"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Literal strings passed to the translation helpers in a template.
    fn template_keys(source: &str) -> Vec<String> {
        let mut keys = Vec::new();
        for marker in [".t(\"", ".tv(\"", ".tv2(\"", ".tv3(\""] {
            for part in source.split(marker).skip(1) {
                if let Some(end) = part.find('"') {
                    keys.push(part[..end].to_string());
                }
            }
        }
        // me.count(n, "one", "many") and me.link("/url", "text")
        for marker in [".count(", ".link("] {
            for part in source.split(marker).skip(1) {
                let call = part.split(')').next().unwrap_or_default();
                keys.extend(call.split('"').skip(1).step_by(2).map(str::to_string));
            }
        }
        keys.retain(|k| k.chars().any(char::is_alphabetic) && !k.starts_with('/'));
        keys
    }

    /// Messages Rust code shows through `alert(lang, "error", "...")`,
    /// `settings_error(lang, "...")` and similar helpers.
    fn rust_message_keys(source: &str) -> Vec<String> {
        let mut keys = template_keys(source);
        let markers = [
            "\"error\",",
            "\"success\",",
            "\"info\",",
            "settings_error(me.lang,",
            "tag_form_error(me.lang,",
            "render_error(",
        ];
        for marker in markers {
            for part in source.split(marker).skip(1) {
                let Some(rest) = part.trim_start().strip_prefix('"') else {
                    continue;
                };
                if let Some(end) = rest.find('"') {
                    keys.push(rest[..end].replace("\\\n", "").to_string());
                }
            }
        }
        keys.retain(|k| k.chars().any(char::is_alphabetic));
        keys
    }

    /// Rust source files, without their tests.
    fn rust_sources(dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if !path.ends_with("i18n") {
                    rust_sources(&path, out);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                let code = source.split("#[cfg(test)]").next().unwrap_or_default();
                out.push(code.to_string());
            }
        }
    }

    /// Every piece of text that needs translating: what templates and Rust
    /// code show, what the theme uses, the password rules, and what any
    /// language file translates.
    fn all_keys() -> HashSet<String> {
        let mut keys: HashSet<String> = HashSet::new();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/admin/templates");
        for entry in std::fs::read_dir(dir).unwrap() {
            let source = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            keys.extend(template_keys(&source));
        }
        let mut sources = Vec::new();
        rust_sources(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
            &mut sources,
        );
        for source in &sources {
            keys.extend(rust_message_keys(source));
        }
        keys.extend(THEME_STRINGS.iter().map(|(_, en)| en.to_string()));
        keys.extend(
            crate::app::security::PASSWORD_RULES
                .iter()
                .map(|rule| rule.text.to_string()),
        );
        for language in LANGUAGES {
            keys.extend(language.translations.iter().map(|(en, _)| en.to_string()));
        }
        keys
    }

    #[test]
    fn every_language_translates_everything() {
        let keys = all_keys();
        for language in &LANGUAGES[1..] {
            let translated: HashSet<&str> =
                language.translations.iter().map(|(en, _)| *en).collect();
            let mut missing: Vec<&String> = keys
                .iter()
                .filter(|k| !translated.contains(k.as_str()))
                .collect();
            missing.sort();
            assert!(
                missing.is_empty(),
                "{}.rs is missing translations for:\n{}",
                language.code,
                missing
                    .iter()
                    .map(|k| format!("    {k:?}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }

    #[test]
    fn translations_keep_their_placeholders_and_are_listed_once() {
        let names = |s: &str| {
            let mut v: Vec<_> = placeholder_names(s)
                .into_iter()
                .map(str::to_string)
                .collect();
            v.sort();
            v
        };
        for language in LANGUAGES {
            let mut seen = HashSet::new();
            for (en, translated) in language.translations {
                assert!(seen.insert(en), "{}.rs lists {en:?} twice", language.code);
                assert_eq!(names(en), names(translated), "{}.rs: {en}", language.code);
                assert!(!translated.trim().is_empty(), "{}.rs: {en}", language.code);
            }
        }
    }

    #[test]
    fn language_codes_are_unique() {
        let codes: HashSet<&str> = LANGUAGES.iter().map(|l| l.code).collect();
        assert_eq!(codes.len(), LANGUAGES.len());
        assert_eq!(Lang::default().code(), "en");
    }

    #[test]
    fn theme_templates_only_use_known_ids() {
        let ids: HashSet<&str> = THEME_STRINGS.iter().map(|(id, _)| *id).collect();
        let themes = concat!(env!("CARGO_MANIFEST_DIR"), "/themes");
        let templates = std::fs::read_dir(themes)
            .unwrap()
            .flatten()
            .filter_map(|theme| std::fs::read_dir(theme.path().join("templates")).ok())
            .flatten();
        for entry in templates {
            let path = entry.unwrap().path();
            let source = std::fs::read_to_string(&path).unwrap();
            for part in source.split("t.").skip(1) {
                let id: String = part
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                // Only `t.` that starts a lookup ({{ t.x }}), not words ending in "t".
                if source.contains(&format!("{{{{ t.{id}"))
                    || source.contains(&format!("{{{{ t.{id} |"))
                {
                    assert!(
                        ids.contains(id.as_str()),
                        "{}: unknown theme string t.{id}",
                        path.display()
                    );
                }
            }
        }
    }

    fn lang(code: &str) -> Lang {
        Lang::parse(code).unwrap()
    }

    #[test]
    fn placeholders_and_plurals_follow_the_language() {
        assert_eq!(
            lang("tr").tv2("Emailed to {n} subscribers on {date}.", 3, "Oct 2"),
            "Oct 2 tarihinde 3 aboneye gönderildi."
        );
        assert_eq!(lang("en").count(1, "{n} post", "{n} posts"), "1 post");
        assert_eq!(lang("en").count(0, "{n} post", "{n} posts"), "0 posts");
        assert_eq!(lang("de").count(2, "{n} post", "{n} posts"), "2 Beiträge");
        // French treats zero as singular.
        assert_eq!(lang("fr").count(0, "{n} post", "{n} posts"), "0 article");
        assert_eq!(lang("tr").t("Something new"), "Something new");
    }

    #[test]
    fn dates_follow_the_language() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        assert_eq!(lang("en").date(date), "Oct 2, 2026");
        assert_eq!(lang("en").day_month(date), "Oct 2");
        assert_eq!(lang("tr").date(date), "2 Eki 2026");
        assert_eq!(lang("tr").day_month(date), "2 Eki");
        assert_eq!(lang("de").date(date), "2. Okt. 2026");
        assert_eq!(lang("ja").date(date), "2026年10月2日");
    }

    #[test]
    fn picks_language_from_browser() {
        assert_eq!(
            Lang::from_accept_language("tr-TR,tr;q=0.9,en;q=0.8"),
            lang("tr")
        );
        assert_eq!(Lang::from_accept_language("pt-BR,pt;q=0.9"), lang("pt"));
        assert_eq!(Lang::from_accept_language("nl-NL,de;q=0.5"), lang("de"));
        assert_eq!(Lang::from_accept_language("ZH-CN"), lang("zh"));
        assert_eq!(Lang::from_accept_language("xx"), Lang::default());
        assert_eq!(Lang::from_accept_language(""), Lang::default());
    }
}
