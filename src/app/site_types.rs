//! The kinds of site Bloogla can start as. Setup asks "What are you making?"
//! and the answer (`site_type` in settings) decides the starter: its theme,
//! and later the features it needs (a shop, docs navigation...).
//!
//! A type is offered once its starter is built (`ready`); the others are
//! shown as "Coming soon".

pub struct SiteType {
    /// Stored in settings, e.g. `blog`.
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Inside a 24×24 SVG (Lucide icons).
    pub icon: &'static str,
    /// The bundled theme a new site of this type starts with.
    pub theme: &'static str,
    pub ready: bool,
}

pub const SITE_TYPES: &[SiteType] = &[
    SiteType {
        id: "blog",
        name: "Blog",
        description: "Posts, tags, comments and a newsletter.",
        icon: r#"<path d="M12 20h9"/><path d="M16.38 3.62a1 1 0 0 1 3 3L7.37 18.64a2 2 0 0 1-.86.5l-2.87.84a.5.5 0 0 1-.62-.62l.84-2.87a2 2 0 0 1 .5-.86z"/>"#,
        theme: "default",
        ready: true,
    },
    SiteType {
        id: "portfolio",
        name: "Portfolio",
        description: "Show your work as a gallery of projects.",
        icon: r#"<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#,
        theme: "portfolio",
        ready: true,
    },
    SiteType {
        id: "docs",
        name: "Documentation",
        description: "Guides and help pages with a sidebar.",
        icon: r#"<path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/>"#,
        theme: "docs",
        ready: true,
    },
    SiteType {
        id: "video",
        name: "Video site",
        description: "Videos from YouTube, Vimeo or your uploads.",
        icon: r#"<path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/><rect x="2" y="6" width="14" height="12" rx="2"/>"#,
        theme: "video",
        ready: true,
    },
    SiteType {
        id: "shop",
        name: "Shop",
        description: "Sell digital or physical products.",
        icon: r#"<path d="M6 2 3 6v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V6l-3-4Z"/><path d="M3 6h18"/><path d="M16 10a4 4 0 0 1-8 0"/>"#,
        theme: "shop",
        ready: false,
    },
];

/// The type with this id, if it's ready to use.
pub fn ready(id: &str) -> Option<&'static SiteType> {
    SITE_TYPES.iter().find(|t| t.id == id && t.ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_types_have_a_bundled_theme() {
        for site_type in SITE_TYPES.iter().filter(|t| t.ready) {
            let theme = format!(
                "{}/themes/{}/theme.toml",
                env!("CARGO_MANIFEST_DIR"),
                site_type.theme
            );
            assert!(std::path::Path::new(&theme).exists(), "{theme} is missing");
        }
        assert!(ready("blog").is_some());
        assert!(ready("nonsense").is_none());
    }
}
