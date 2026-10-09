# Translating Bloogla

Bloogla speaks English and Türkçe today. Every language added means more
people can run a website in their own words: the admin panel, the themes and
the emails all use it. **You don't need to know Rust or install anything** to
help.

- [Three ways to help](#three-ways-to-help)
- [Adding a language](#adding-a-language)
- [Writing good translations](#writing-good-translations)

## Three ways to help

1. **Fix a word.** Spotted something odd in an existing translation? Open
   [`src/i18n/tr.rs`](../src/i18n/tr.rs) on GitHub, click the pencil
   (**Edit this file**), change it and choose **Propose changes**. Or just
   [tell us](https://github.com/dincertekin/bloogla/issues/new?template=translation.yml).
2. **Add a language.** Follow the steps below. It's about 490 short texts, so
   it's fine to send a part and continue later.
3. **Review one.** Native speakers checking a new language before it's
   released make the biggest difference. Say hello in a
   [translation issue](https://github.com/dincertekin/bloogla/issues/new?template=translation.yml).

## Adding a language

A language is one file in [`src/i18n/`](../src/i18n/). Each line pairs the
English text with yours:

```rust
("Save Changes", "Değişiklikleri Kaydet"),
("by {name}", "{name} tarafından"),
```

**In your browser (no installs):**

1. Open [`src/i18n/tr.rs`](../src/i18n/tr.rs) on GitHub and copy all of it
   (the copy button above the file).
2. Go to the [`src/i18n`](../src/i18n/) folder, choose **Add file → Create new
   file**, and name it after your language's
   [two-letter code](https://en.wikipedia.org/wiki/List_of_ISO_639_language_codes),
   for example `nl.rs` for Dutch. Paste the copy.
3. Change the top of the file to your language:

   ```rust
   //! Nederlands (Dutch).
   ...
       code: "nl",
       name: "Nederlands",          // your language's own name
       months: [                    // short month names, January first
           "jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
       ],
       date: "{d} {m} {y}",         // how a date looks: 9 okt 2026
       day_month: "{d} {m}",        // and without the year: 9 okt
   ```

   `{d}` is the day, `{m}` the month name and `{y}` the year; use them in
   your language's order (English writes `{m} {d}, {y}`, "Oct 9, 2026").
4. Translate the **right-hand** text of each line. Leave the left-hand English
   exactly as it is: it's how Bloogla finds your translation.
5. Open [`src/i18n/mod.rs`](../src/i18n/mod.rs) and add your language in two
   places (the pencil again):

   ```rust
   mod en;
   mod nl;      // ← add
   mod tr;
   ...
   pub const LANGUAGES: &[&Language] = &[&en::LANGUAGE, &nl::LANGUAGE, &tr::LANGUAGE];
   ```

6. Choose **Propose changes** and open a pull request. Bloogla's tests run on
   it automatically and list anything missing or mistyped; you'll get help
   with any of it.

**On your computer** (if you have [Rust](https://rustup.rs)): do the same
steps in your editor, then run `cargo test`. It lists every text your file
doesn't translate yet, and `cargo run` shows your language right away: pick
it in **Profile → Language**.

## Writing good translations

- **Keep the `{placeholders}`** exactly as written, like `{name}` or `{n}`;
  Bloogla puts the real name or number there. You can move them to wherever
  your sentence needs them.
- **Write like the English does:** short, friendly, plain. Bloogla is for
  people who aren't developers, so prefer everyday words to technical ones
  ("address" rather than "URL", "sign in" rather than "authenticate").
- **Use the informal or formal "you"** your language uses in friendly apps,
  and keep it the same everywhere.
- **Counts:** English has a text for one and for many (`"{n} comment"` and
  `"{n} comments"`). If your language changes words in other ways (Russian,
  Arabic, Polish…), choose wording that works for any number, like
  "Comments: {n}". If it uses the singular for zero too (French), tell us in
  the pull request and we'll switch the rule.
- **Keep names as they are:** Bloogla, Gmail, Docker, YouTube.
- **Not sure about a text?** Search for it in the code to see where it's
  used, or ask in your pull request.

Thank you! Your name goes in the release notes of the version your language
first appears in.
