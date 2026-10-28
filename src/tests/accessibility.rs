//! Things people with a keyboard or a screen reader rely on, checked so they
//! don't break again. (The whole site is also checked against WCAG 2.2 AA
//! with axe before releases with new screens; see CONTRIBUTING.md.)

use super::TestSite;

/// The rules of one CSS selector in the admin stylesheet.
fn admin_css_rule(selector: &str) -> String {
    let css = include_str!("../../admin/static/css/style.css");
    let start = css
        .find(&format!("\n{selector} {{"))
        .unwrap_or_else(|| panic!("{selector} is in style.css"));
    let rest = &css[start..];
    rest[..rest.find('}').unwrap()].to_string()
}

#[test]
fn closed_pop_ups_are_hidden_from_the_keyboard_and_screen_readers() {
    // See-through (opacity) alone leaves their buttons in the Tab order and
    // their text read out on every page.
    assert!(admin_css_rule(".modal-overlay").contains("visibility: hidden"));
    assert!(admin_css_rule(".modal-overlay.active").contains("visibility: visible"));
}

#[tokio::test]
async fn the_editor_has_a_heading_and_named_side_areas() {
    let mut site = TestSite::with_owner().await;
    let editor = site.get("/admin/posts/new").await.body;
    assert!(editor.contains(r#"<h1 class="visually-hidden">New Post</h1>"#));
    // Two side areas (the menu and the post's settings) need telling apart.
    assert!(editor.contains(r#"<aside class="sidebar" aria-label="Menu">"#));
}

#[tokio::test]
async fn the_brief_themes_page_heading_is_inside_main() {
    let mut site = TestSite::with_owner().await;
    site.post("/admin/themes/activate", &[("theme", "brief")])
        .await;
    site.post(
        "/admin/posts",
        &[
            ("title", "Basil"),
            ("content", "Green."),
            ("status", "published"),
            ("new_tags", "Garden"),
        ],
    )
    .await;
    site.new_visitor();
    let page = site.get("/tag/garden").await.body;
    let main = page.find(r#"<main id="main">"#).expect("main");
    let band = page.find(r#"<section class="band">"#).expect("band");
    // "Skip to content" leads to <main>, so the page's heading must be in it.
    assert!(main < band);
}
