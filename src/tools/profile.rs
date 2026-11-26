use rust_mcp_sdk::schema::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{normalize_text, tool};

pub(super) const NAME: &str = "draft_profile_update";

const HEADLINE_LIMIT: usize = 220;
const ABOUT_LIMIT: usize = 2600;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProfileField {
    Headline,
    About,
}

impl ProfileField {
    const NAMES: [&'static str; 2] = ["headline", "about"];

    fn details(&self) -> (&'static str, usize, &'static str) {
        match self {
            Self::Headline => (
                Self::NAMES[0],
                HEADLINE_LIMIT,
                "https://www.linkedin.com/in/me/edit/topcard/",
            ),
            Self::About => (
                Self::NAMES[1],
                ABOUT_LIMIT,
                "https://www.linkedin.com/in/me/edit/about/",
            ),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileDraftArgs {
    field: ProfileField,
    current_text: Option<String>,
    proposed_text: String,
}

#[derive(Serialize)]
pub(super) struct ProfileDraftResult {
    field: &'static str,
    proposed_text: String,
    edit_url: &'static str,
}

pub(super) fn definition() -> Tool {
    tool(
        NAME,
        "Format a local draft for the LinkedIn headline or About section and return the LinkedIn edit link for manual editing.",
        json!({
            "field": {"type": "string", "enum": ProfileField::NAMES},
            "current_text": {"type": ["string", "null"], "description": "Optional existing text supplied by the user."},
            "proposed_text": {"type": "string", "description": "Draft text. Limits in UTF-16 code units are 220 for headline and 2600 for about."}
        }),
        &["field", "proposed_text"],
        true,
    )
}

pub(super) fn draft_profile_update(
    args: ProfileDraftArgs,
) -> Result<ProfileDraftResult, &'static str> {
    let (field, limit, edit_url) = args.field.details();
    let proposed_text = normalize_profile_text(&args.proposed_text, &args.field);
    if proposed_text.is_empty() {
        return Err("proposed_text must contain nonblank text");
    }
    if !valid_profile_text(&args.proposed_text, &proposed_text, limit) {
        return Err(
            "proposed_text exceeds the field limit or contains unsupported control characters",
        );
    }
    if let Some(current) = args.current_text.as_deref() {
        let normalized = normalize_profile_text(current, &args.field);
        if !valid_profile_text(current, &normalized, limit) {
            return Err(
                "current_text exceeds the field limit or contains unsupported control characters",
            );
        }
    }
    Ok(ProfileDraftResult {
        field,
        proposed_text,
        edit_url,
    })
}

fn normalize_profile_text(text: &str, field: &ProfileField) -> String {
    let text = normalize_text(text);
    if matches!(field, ProfileField::Headline) {
        return text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    text
}

fn valid_profile_text(original: &str, normalized: &str, limit: usize) -> bool {
    normalized.encode_utf16().take(limit + 1).count() <= limit
        && !original
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
}
