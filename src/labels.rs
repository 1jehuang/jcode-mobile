//! Display labels shared with Jcode Desktop's composer pills, so the phone
//! reads `Opus 5.5`, `low`, `Anthropic · OAuth` exactly like the desktop.
//! Ported from `jcode_provider_core::model_names::pretty_model_display_name`
//! and desktop's `pretty_model_label` / `account_method_label`.

/// `claude-opus-4-8` reads `Opus 4.8` (the provider pill already says Anthropic).
pub fn pretty_model(model: Option<&str>) -> String {
    let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) else {
        return "Choose model".into();
    };
    let pretty = pretty_model_display_name(model);
    match pretty.strip_prefix("Claude ") {
        Some(rest) if !rest.trim().is_empty() => rest.to_string(),
        _ => pretty,
    }
}

fn pretty_model_display_name(model: &str) -> String {
    let (core, bracket) = match model.rfind('[') {
        Some(open) if model.ends_with(']') && open + 2 < model.len() => {
            let inner = &model[open + 1..model.len() - 1];
            let inner = if inner.eq_ignore_ascii_case("1m") { "1M".to_string() } else { inner.to_string() };
            (&model[..open], Some(inner))
        }
        _ => (model, None),
    };
    let (core, date) = match core.rsplit_once('-') {
        Some((head, tail)) if !head.is_empty() && tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()) => {
            (head, Some(format!("{}-{}-{}", &tail[..4], &tail[4..6], &tail[6..])))
        }
        _ => (core, None),
    };
    let lower = core.to_ascii_lowercase();
    let mut pretty = if lower.starts_with("gpt-") {
        versioned_family("GPT", core)
    } else if lower.starts_with("claude-") {
        claude(core)
    } else {
        core.split('-').map(title_word).collect::<Vec<_>>().join(" ")
    };
    let markers: Vec<String> = date.into_iter().chain(bracket).collect();
    if !markers.is_empty() {
        pretty.push_str(&format!(" ({})", markers.join(", ")));
    }
    pretty
}

fn versioned_family(family: &str, core: &str) -> String {
    let Some((_, rest)) = core.split_once('-') else { return family.into() };
    let mut parts = rest.split('-');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with(|c: char| c.is_ascii_digit()) {
        return core.split('-').map(title_word).collect::<Vec<_>>().join(" ");
    }
    let mut out = format!("{family}-{version}");
    for p in parts {
        out.push(' ');
        out.push_str(&title_word(p));
    }
    out
}

fn claude(core: &str) -> String {
    let parts: Vec<&str> = core.split('-').collect();
    let mut words = Vec::new();
    let mut i = 0;
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    while i < parts.len() {
        if digits(parts[i]) && i + 1 < parts.len() && digits(parts[i + 1]) {
            words.push(format!("{}.{}", parts[i], parts[i + 1]));
            i += 2;
        } else {
            words.push(title_word(parts[i]));
            i += 1;
        }
    }
    words.join(" ")
}

fn title_word(word: &str) -> String {
    if word.chars().any(|c| c.is_ascii_digit()) {
        return word.into();
    }
    let lower = word.to_ascii_lowercase();
    if ["tts", "stt", "api", "vl", "ocr", "id", "gpt", "oss", "glm"].contains(&lower.as_str()) {
        return lower.to_ascii_uppercase();
    }
    let mut c = word.chars();
    c.next().map(|f| format!("{}{}", f.to_ascii_uppercase(), c.as_str())).unwrap_or_default()
}

/// Desktop's provider names: `claude-oauth` and `Claude` read `Anthropic`.
pub fn pretty_provider(provider: &str) -> String {
    let lower = provider.trim().to_ascii_lowercase();
    let base = lower
        .trim_end_matches(" api key")
        .trim_end_matches(" api")
        .trim_end_matches(" oauth")
        .trim_end_matches("-api-key")
        .trim_end_matches("-api")
        .trim_end_matches("-oauth")
        .trim_end_matches("-key");
    match base {
        "anthropic" | "claude" => "Anthropic".into(),
        "openai" | "chatgpt" => "OpenAI".into(),
        "openrouter" => "OpenRouter".into(),
        "copilot" | "github copilot" | "github-copilot" => "Copilot".into(),
        "gemini" | "google" | "code-assist" => "Gemini".into(),
        "antigravity" => "Antigravity".into(),
        "cursor" => "Cursor".into(),
        "bedrock" | "aws bedrock" | "aws-bedrock" => "Bedrock".into(),
        "xai" | "grok" => "xAI".into(),
        "jcode" => "Jcode".into(),
        _ => provider.trim().to_string(),
    }
}

/// `Anthropic · OAuth`. The auth method comes from the active route.
pub fn account_label(provider: Option<&str>, api_method: Option<&str>) -> String {
    let provider = provider.map(str::trim).filter(|p| !p.is_empty()).map(pretty_provider);
    let method = api_method.map(str::trim).filter(|m| !m.is_empty()).and_then(|m| {
        let m = m.to_ascii_lowercase();
        if m.contains("oauth") || m == "claude" || m == "openai" {
            Some("OAuth".to_string())
        } else if m.contains("api") || m.contains("key") {
            Some("API key".to_string())
        } else {
            None
        }
    });
    match (provider, method) {
        (Some(p), Some(m)) => format!("{p} · {m}"),
        (Some(p), None) => p,
        (None, Some(m)) => m,
        (None, None) => "Accounts".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_desktop_labels() {
        assert_eq!(pretty_model(Some("claude-opus-5-5")), "Opus 5.5");
        assert_eq!(pretty_model(Some("claude-sonnet-4-5-20250929")), "Sonnet 4.5 (2025-09-29)");
        assert_eq!(pretty_model(Some("gpt-5.1-codex-max")), "GPT-5.1 Codex Max");
        assert_eq!(pretty_model(None), "Choose model");
        assert_eq!(account_label(Some("Claude"), Some("claude-oauth")), "Anthropic · OAuth");
        assert_eq!(account_label(Some("openai"), Some("openai-api")), "OpenAI · API key");
    }
}
