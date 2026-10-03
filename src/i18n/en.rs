//! English: the language the interface is written in, so it needs no translations.

use super::{singular_if_one, Language};

pub const LANGUAGE: Language = Language {
    code: "en",
    name: "English",
    months: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    date: "{m} {d}, {y}",
    day_month: "{m} {d}",
    is_singular: singular_if_one,
    translations: &[],
};
