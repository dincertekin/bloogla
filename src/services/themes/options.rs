//! Theme options: settings a theme offers in its `theme.toml` (a color, a
//! footer text, a switch...). Admins change them in Admin → Themes →
//! Customize, and the theme's templates read them as `theme_options.<name>`.
//!
//! ```toml
//! [[options]]
//! name = "accent_color"
//! label = "Accent color"
//! type = "color"          # text, textarea, color, checkbox, select or image
//! default = "#2563eb"
//! hint = "Links and buttons."
//! ```
//!
//! Values are stored in the `settings` table as `theme.<theme>.<name>`.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// One option a theme offers.
#[derive(Debug, Clone, Deserialize)]
pub struct ThemeOption {
    /// Used in templates: `theme_options.<name>`.
    pub name: String,
    pub label: String,
    #[serde(rename = "type", default)]
    pub kind: OptionKind,
    pub default: Option<toml::Value>,
    /// A short explanation shown under the field.
    #[serde(default)]
    pub hint: String,
    /// The choices of a `select`.
    #[serde(default)]
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionKind {
    /// One line of text.
    #[default]
    Text,
    /// Several lines of text.
    Textarea,
    /// A color like `#2563eb`.
    Color,
    /// On or off.
    Checkbox,
    /// One of `choices`.
    Select,
    /// An image address, usually from the media library.
    Image,
}

impl ThemeOption {
    /// The value used until the admin changes it.
    pub fn default_value(&self) -> String {
        match (&self.default, self.kind) {
            (Some(toml::Value::String(text)), _) => text.clone(),
            (Some(toml::Value::Boolean(on)), _) => on.to_string(),
            (Some(other), _) => other.to_string(),
            (None, OptionKind::Checkbox) => "false".into(),
            (None, OptionKind::Color) => "#000000".into(),
            (None, OptionKind::Select) => self.choices.first().cloned().unwrap_or_default(),
            (None, _) => String::new(),
        }
    }

    /// Check a value typed by an admin, and tidy it up (trimmed, colors in
    /// lowercase). Errors are messages for the admin.
    pub fn clean(&self, input: &str) -> Result<String, &'static str> {
        let value = input.trim();
        match self.kind {
            OptionKind::Text if value.chars().count() > 500 => Err("This text is too long."),
            OptionKind::Textarea if value.chars().count() > 5000 => Err("This text is too long."),
            OptionKind::Text | OptionKind::Textarea => Ok(value.to_string()),
            OptionKind::Color => {
                let is_color = value.len() == 7
                    && value.starts_with('#')
                    && value[1..].chars().all(|c| c.is_ascii_hexdigit());
                if is_color {
                    Ok(value.to_lowercase())
                } else {
                    Err("Use a color like #2563eb.")
                }
            }
            OptionKind::Checkbox => Ok((value == "true" || value == "on").to_string()),
            OptionKind::Select if self.choices.iter().any(|c| c == value) => Ok(value.to_string()),
            OptionKind::Select => Err("Pick one of the choices."),
            OptionKind::Image => {
                let safe = !value.contains(['"', '\'', '<', '>', '(', ')', ' ', '\\'])
                    && (value.is_empty()
                        || value.starts_with('/')
                        || value.starts_with("https://"));
                if safe {
                    Ok(value.to_string())
                } else {
                    Err("Choose an image from the media library or use an https:// address.")
                }
            }
        }
    }

    pub fn is_checkbox(&self) -> bool {
        self.kind == OptionKind::Checkbox
    }

    pub fn is_textarea(&self) -> bool {
        self.kind == OptionKind::Textarea
    }

    pub fn is_color(&self) -> bool {
        self.kind == OptionKind::Color
    }

    pub fn is_select(&self) -> bool {
        self.kind == OptionKind::Select
    }

    pub fn is_image(&self) -> bool {
        self.kind == OptionKind::Image
    }
}

/// Where an option's value is stored in the `settings` table.
pub fn storage_key(theme: &str, name: &str) -> String {
    format!("theme.{theme}.{name}")
}

/// Find mistakes in a theme's options, so a theme with a broken option
/// list isn't used.
pub fn check(options: &[ThemeOption]) -> Result<(), String> {
    let mut names = HashSet::new();
    for option in options {
        let name = &option.name;
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            return Err(format!(
                "option name \"{name}\" may only use lowercase letters, numbers and _"
            ));
        }
        if !names.insert(name) {
            return Err(format!("option \"{name}\" is listed twice"));
        }
        if option.kind == OptionKind::Select && option.choices.is_empty() {
            return Err(format!("option \"{name}\" is a select without choices"));
        }
        if let Err(problem) = option.clean(&option.default_value()) {
            return Err(format!("the default of option \"{name}\": {problem}"));
        }
    }
    Ok(())
}

/// The current value of an option: the saved one, or the default.
pub fn current(theme: &str, option: &ThemeOption, saved: &HashMap<String, String>) -> String {
    saved
        .get(&storage_key(theme, &option.name))
        .and_then(|value| option.clean(value).ok())
        // `check` made sure the default is valid when the theme was loaded.
        .unwrap_or_else(|| option.clean(&option.default_value()).unwrap_or_default())
}

/// Every option of a theme for its templates: checkboxes as true/false,
/// everything else as text.
pub fn for_templates(
    theme: &str,
    options: &[ThemeOption],
    saved: &HashMap<String, String>,
) -> serde_json::Map<String, serde_json::Value> {
    options
        .iter()
        .map(|option| {
            let value = current(theme, option, saved);
            let value = if option.is_checkbox() {
                serde_json::Value::Bool(value == "true")
            } else {
                serde_json::Value::String(value)
            };
            (option.name.clone(), value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(toml_text: &str) -> Vec<ThemeOption> {
        #[derive(Deserialize)]
        struct File {
            options: Vec<ThemeOption>,
        }
        toml::from_str::<File>(toml_text).unwrap().options
    }

    #[test]
    fn values_are_checked_and_defaults_fill_the_gaps() {
        let list = options(
            r##"
            [[options]]
            name = "accent"
            label = "Accent"
            type = "color"
            default = "#2563EB"
            [[options]]
            name = "credit"
            label = "Credit"
            type = "checkbox"
            default = true
            [[options]]
            name = "layout"
            label = "Layout"
            type = "select"
            choices = ["wide", "narrow"]
            [[options]]
            name = "logo"
            label = "Logo"
            type = "image"
            "##,
        );
        assert!(check(&list).is_ok());
        let [accent, _, layout, logo] = &list[..] else {
            panic!()
        };
        assert_eq!(accent.clean(" #ABCDEF ").unwrap(), "#abcdef");
        assert!(accent.clean("red; } body { display: none").is_err());
        assert!(layout.clean("huge").is_err());
        assert!(logo.clean("/uploads/a.png").is_ok());
        assert!(logo.clean("javascript:alert(1)").is_err());
        assert!(logo.clean("/a.png\" onerror=\"x").is_err());

        // A bad saved value falls back to the default.
        let saved = HashMap::from([
            ("theme.t.accent".to_string(), "not a color".to_string()),
            ("theme.t.credit".to_string(), "false".to_string()),
        ]);
        let values = for_templates("t", &list, &saved);
        assert_eq!(values["accent"], "#2563eb");
        assert_eq!(values["credit"], false);
        assert_eq!(values["layout"], "wide");
    }

    #[test]
    fn mistakes_in_theme_toml_are_found() {
        let twice = options(
            "[[options]]\nname = \"a\"\nlabel = \"A\"\n[[options]]\nname = \"a\"\nlabel = \"B\"",
        );
        assert!(check(&twice).is_err());
        let bad_name = options("[[options]]\nname = \"Bad Name\"\nlabel = \"A\"");
        assert!(check(&bad_name).is_err());
        let no_choices = options("[[options]]\nname = \"a\"\nlabel = \"A\"\ntype = \"select\"");
        assert!(check(&no_choices).is_err());
    }
}
