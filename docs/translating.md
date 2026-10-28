# Translating Bloogla

Bloogla speaks English, Deutsch, Français, Lietuvių and Türkçe. A new
language changes the admin panel, the themes and the emails. **You don't
need to program or install anything.**

- [Ways to help](#three-ways-to-help)
- [Adding a language](#adding-a-language)
- [Writing good translations](#writing-good-translations)

## Three ways to help

1. **Fix a word.** Open the language's file in
   [`src/i18n/`](../src/i18n/) on GitHub, click the pencil, change it, and
   choose **Propose changes**. Or just
   [tell us](https://github.com/dincertekin/bloogla/issues/new?template=translation.yml).
2. **Add a language.** See below. It's about 700 short texts; sending part
   of it and finishing later is fine.
3. **Check one.** A native speaker reading a new language before it's
   released helps the most. Say hello in a
   [translation issue](https://github.com/dincertekin/bloogla/issues/new?template=translation.yml).

## Adding a language

A language is one file. Each line pairs the English text with yours:

```rust
("Save Changes", "Değişiklikleri Kaydet"),
("by {name}", "{name} tarafından"),
```

In your browser:

1. Open [`src/i18n/tr.rs`](../src/i18n/tr.rs) on GitHub and copy all of it.
2. In [`src/i18n`](../src/i18n/), choose **Add file → Create new file**. Name
   it with your language's
   [two-letter code](https://en.wikipedia.org/wiki/List_of_ISO_639_language_codes),
   like `nl.rs` for Dutch, and paste.
3. Change the top of the file:

   ```rust
   //! Nederlands (Dutch).
   ...
       code: "nl",
       name: "Nederlands",      // your language's own name
       months: [                // short month names, January first
           "jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
       ],
       date: "{d} {m} {y}",     // a date: 9 okt 2026
       day_month: "{d} {m}",    // without the year: 9 okt
   ```

4. Translate the **right-hand** text of each line. Don't change the English
   on the left: it's how Bloogla finds your text.
5. In [`src/i18n/mod.rs`](../src/i18n/mod.rs), add your language in two
   places:

   ```rust
   mod nl;   // next to the other `mod` lines
   ...
   pub const LANGUAGES: &[&Language] = &[&en::LANGUAGE, /* ... */ &nl::LANGUAGE];
   ```

6. Choose **Propose changes** and open a pull request. The tests check it and
   list anything missing. We'll help with the rest.

With [Rust](https://rustup.rs) on your computer, `cargo test` lists what's
missing, and `cargo run` shows your language (**Profile → Language**).

## Writing good translations

- **Keep `{placeholders}`** like `{name}` and `{n}` exactly as written. You
  can move them to where your sentence needs them.
- **Short, friendly and plain.** Bloogla is for people who aren't
  developers: "address", not "URL"; "sign in", not "authenticate".
- **Always the polite "you"**: *Sie*, *vous*, *siz*, *jūs*. Warm, but never
  informal.
- **Counts:** English has one text for one and one for many (`"{n} comment"`,
  `"{n} comments"`). If your language works differently, use wording that
  fits any number ("Comments: {n}"), and tell us in the pull request.
- **Keep names** as they are: Bloogla, Gmail, Docker, YouTube.

Thank you! Your name goes in the release notes of the version your language
first appears in.
