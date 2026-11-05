//! Model picker support: turn a [`ModelRouteInfo`] into the `set_model` spec
//! that selects exactly that model on exactly that provider/credential.
//! Mirrors `jcode_provider_core::RouteSelection::routed_model_spec`, which the
//! TUI and desktop pickers use, without pulling in the provider stack.

use jcode_sdk::ModelRouteInfo;

/// `set_model` spec for a route, e.g. `claude-oauth:claude-opus-5-5`,
/// `openai-api:gpt-6`, `anthropic/claude-opus-5-5@Anthropic` (OpenRouter).
pub fn route_spec(route: &ModelRouteInfo) -> String {
    let model = route.model.trim();
    let method = route.api_method.trim();
    // Pinned OAuth account: `claude-oauth@work`.
    if let Some((base, label)) = method.split_once('@')
        && matches!(base, "claude-oauth" | "openai-oauth")
        && !label.is_empty()
        && !label.contains([':', '@'])
    {
        return format!("{base}@{label}:{model}");
    }
    let lower = method.to_ascii_lowercase();
    match lower.as_str() {
        "claude" | "anthropic" | "claude-oauth" | "anthropic-oauth" => format!("claude-oauth:{model}"),
        "claude-api" | "anthropic-api" | "anthropic-api-key" | "claude-api-key" | "anthropic-key"
        | "claude-key" | "api-key" => format!("claude-api:{model}"),
        "openai" | "openai-oauth" => format!("openai-oauth:{model}"),
        "openai-api" | "openai-api-key" | "openai-key" | "openai-apikey" | "openai-platform"
        | "platform-openai" => format!("openai-api:{model}"),
        "copilot" => format!("copilot:{model}"),
        "cursor" => format!("cursor:{model}"),
        "bedrock" => format!("bedrock:{model}"),
        "https" => format!("antigravity:{model}"),
        "grok-build" | "grok-build-acp" => {
            format!("grok-build:{}", model.strip_prefix("grok-build:").unwrap_or(model))
        }
        "openrouter" => {
            let provider = route.provider.trim();
            let id = if model.contains('/') {
                model.to_string()
            } else if model.starts_with("claude") {
                format!("anthropic/{model}")
            } else if model.starts_with("gpt") || model.starts_with('o') && model[1..].starts_with(char::is_numeric) {
                format!("openai/{model}")
            } else {
                model.to_string()
            };
            if provider.is_empty() || provider.eq_ignore_ascii_case("auto") || model.contains('@') {
                id
            } else {
                format!("{id}@{provider}")
            }
        }
        _ => match lower.split_once(':') {
            Some(("openai-compatible", profile)) if !profile.trim().is_empty() => {
                format!("{}:{model}", profile.trim())
            }
            _ => model.to_string(),
        },
    }
}

/// Short label for the auth method, shown next to the provider.
pub fn method_label(route: &ModelRouteInfo) -> &'static str {
    let m = route.api_method.to_ascii_lowercase();
    if m.contains("oauth") || m == "claude" || m == "openai" {
        "subscription"
    } else if m.contains("api") || m.contains("key") {
        "API key"
    } else if m == "jcode-subscription" {
        "Jcode"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(model: &str, provider: &str, method: &str) -> ModelRouteInfo {
        ModelRouteInfo {
            model: model.into(),
            provider: provider.into(),
            api_method: method.into(),
            available: true,
            detail: String::new(),
            usage: None,
        }
    }

    #[test]
    fn specs_match_provider_core() {
        assert_eq!(route_spec(&r("claude-opus-5-5", "Anthropic", "claude-oauth")), "claude-oauth:claude-opus-5-5");
        assert_eq!(route_spec(&r("claude-opus-5-5", "Anthropic", "api-key")), "claude-api:claude-opus-5-5");
        assert_eq!(route_spec(&r("gpt-6", "OpenAI", "openai-oauth")), "openai-oauth:gpt-6");
        assert_eq!(route_spec(&r("claude-opus-5-5", "Anthropic", "claude-oauth@work")), "claude-oauth@work:claude-opus-5-5");
        assert_eq!(route_spec(&r("claude-opus-5-5", "Fireworks", "openrouter")), "anthropic/claude-opus-5-5@Fireworks");
        assert_eq!(route_spec(&r("claude-opus-5-5", "auto", "openrouter")), "anthropic/claude-opus-5-5");
        assert_eq!(route_spec(&r("glm-5", "Jcode", "jcode-subscription")), "glm-5");
        assert_eq!(route_spec(&r("grok-4.6", "xAI", "grok-build")), "grok-build:grok-4.6");
    }
}
