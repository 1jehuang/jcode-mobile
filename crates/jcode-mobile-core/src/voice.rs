//! Draft composition for client-owned Nari realtime dictation.

/// Preserve the user's current typing. Dictation never submits a message.
pub fn append(draft: &str, text: &str) -> String {
    if draft.is_empty() || draft.ends_with(char::is_whitespace) {
        format!("{draft}{text}")
    } else {
        format!("{draft} {text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_draft() {
        assert_eq!(append("", "Hello"), "Hello");
        assert_eq!(append("Typed", "spoken"), "Typed spoken");
        assert_eq!(append("Typed\n", "spoken"), "Typed\nspoken");
        assert_eq!(append("Typed ", "spoken"), "Typed spoken");
        assert_eq!(append("Typed\t", "spoken"), "Typed\tspoken");
    }

    #[test]
    fn preserves_unicode_draft_and_transcript() {
        assert_eq!(append("東京", "école"), "東京 école");
        assert_eq!(append("Typed\u{2003}", "spoken"), "Typed\u{2003}spoken");
        assert_eq!(append("", ""), "");
    }
}
