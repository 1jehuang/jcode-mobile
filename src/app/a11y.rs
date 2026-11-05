//! VoiceOver helpers. GPUI only exposes elements with an explicit role to
//! AccessKit (and so to the iOS bridge), so icon-only controls need a role
//! plus a spoken label. Text-bearing buttons get their label merged from
//! their children by the bridge.

use gpui::{Role, SharedString, StatefulInteractiveElement};

pub(crate) trait A11yExt: StatefulInteractiveElement {
    /// Mark as a button VoiceOver reads as `label`.
    fn a11y_button(self, label: impl Into<SharedString>) -> Self {
        self.role(Role::Button).aria_label(label)
    }
    /// Mark as a button whose spoken label comes from its text children.
    fn a11y_text_button(self) -> Self {
        self.role(Role::Button)
    }
}

impl<T: StatefulInteractiveElement> A11yExt for T {}
