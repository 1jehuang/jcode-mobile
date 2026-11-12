//! Default directory for new mobile sessions (Settings → "New sessions start in").

use gpui::{AnyElement, Context, div, prelude::*, px};
use jcode_mobile_core::PrefsStore;

use super::MobileApp;
use crate::theme::Theme;

impl MobileApp {
    /// Working directory for a connection. An explicit choice wins. Resumed and
    /// forked sessions keep their own directory. Otherwise a brand-new session
    /// starts in the user's default, or the server default when unset.
    pub(super) fn new_session_dir(&self, resume: Option<&str>, explicit: Option<String>, fork_from: Option<&str>) -> Option<String> {
        if explicit.is_some() {
            return explicit;
        }
        if resume.is_some() || fork_from.is_some() {
            return None;
        }
        self.prefs.default_dir.clone()
    }

    /// Persist the default. Empty clears it back to the server default.
    pub(super) fn set_default_dir(&mut self, dir: &str, cx: &mut Context<Self>) {
        let before = self.prefs.default_dir.clone();
        self.prefs.set_default_dir(Some(dir));
        let shown = self.prefs.default_dir.clone().unwrap_or_default();
        self.default_dir_input.update(cx, |t, cx| {
            if t.text() != shown {
                t.set_text(shown.clone(), cx);
            }
        });
        if self.prefs.default_dir != before {
            crate::haptics::play(crate::haptics::Haptic::Selection);
            if !self.offline {
                let _ = PrefsStore::default_location().save(&self.prefs);
            }
        }
        cx.notify();
    }

    /// Settings block: field, Save, "Use current" and Reset.
    pub(super) fn render_default_dir(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let typed = self.default_dir_input.read(cx).text().to_string();
        let saved = self.prefs.default_dir.clone().unwrap_or_default();
        let dirty = typed.trim().trim_end_matches('/') != saved.trim_end_matches('/');
        let invalid = dirty && !typed.trim().is_empty() && jcode_mobile_core::prefs::normalize_dir(&typed).is_none();
        let current = self.session.working_dir.clone().filter(|d| Some(d.trim_end_matches('/')) != self.prefs.default_dir.as_deref());
        let chip = |id: &'static str, text: &'static str| {
            div().id(id).debug_selector(move || id.into()).px(px(10.)).py(px(6.)).rounded(px(10.))
                .bg(t.surface).cursor_pointer().child(super::label(text, t.text_strong, 12.))
        };
        let help = if invalid {
            ("Use an absolute path, like /home/you/project.", t.error)
        } else if saved.is_empty() {
            ("New sessions start in the cloud machine's home directory.", t.text_muted)
        } else {
            ("New sessions start here unless you pick another directory.", t.text_muted)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(super::label("New sessions start in", t.text_strong, 13.))
            .child(
                div()
                    .flex().items_center().gap(px(8.)).pl(px(12.)).pr(px(4.)).min_h(px(40.))
                    .rounded(px(12.)).bg(t.field).border_1().border_color(if invalid { t.error } else { t.input_border })
                    .child(div().id("default-dir-input").debug_selector(|| "default-dir-input".into()).flex_1().min_w_0()
                        .child(self.default_dir_input.clone()))
                    .when(dirty && !invalid, |d| d.child(chip("default-dir-save", "Save").on_click(cx.listener(|this, _, window, cx| {
                        let dir = this.default_dir_input.read(cx).text().to_string();
                        this.set_default_dir(&dir, cx);
                        window.blur(cx);
                        window.dismiss_virtual_keyboard();
                    })))),
            )
            .child(super::label(help.0, help.1, 11.))
            .child(
                div().flex().gap(px(8.))
                    .when_some(current, |d, dir| d.child(chip("default-dir-current", "Use current").on_click(cx.listener(move |this, _, _, cx| {
                        this.set_default_dir(&dir, cx);
                    }))))
                    .when(!saved.is_empty(), |d| d.child(chip("default-dir-reset", "Reset").on_click(cx.listener(|this, _, _, cx| {
                        this.set_default_dir("", cx);
                    })))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::super::account_coverage::{account, draw, mount, settle_settings_fixture, tap};
    use gpui::TestAppContext;

    #[gpui::test]
    fn new_sessions_use_default_but_explicit_resume_and_fork_do_not(cx: &mut TestAppContext) {
        let (view, cx) = mount(cx);
        view.update(cx, |app, cx| {
            assert_eq!(app.new_session_dir(None, None, None), None, "unset keeps server default");
            app.set_default_dir("  /home/jeremy/jcode-mobile/ ", cx);
            assert_eq!(app.prefs.default_dir.as_deref(), Some("/home/jeremy/jcode-mobile"));
            assert_eq!(app.default_dir_input.read(cx).text(), "/home/jeremy/jcode-mobile");
            assert_eq!(app.new_session_dir(None, None, None).as_deref(), Some("/home/jeremy/jcode-mobile"));
            assert_eq!(app.new_session_dir(None, Some("/other".into()), None).as_deref(), Some("/other"));
            assert_eq!(app.new_session_dir(Some("session_x"), None, None), None, "resume keeps its dir");
            assert_eq!(app.new_session_dir(None, None, Some("session_x")), None, "fork keeps its dir");
        });
    }

    #[gpui::test]
    fn invalid_or_empty_input_clears_rather_than_storing_garbage(cx: &mut TestAppContext) {
        let (view, cx) = mount(cx);
        view.update(cx, |app, cx| {
            app.set_default_dir("/repo", cx);
            app.set_default_dir("relative/path", cx);
            assert_eq!(app.prefs.default_dir, None);
            assert_eq!(app.default_dir_input.read(cx).text(), "");
            app.set_default_dir("~/proj", cx);
            assert_eq!(app.prefs.default_dir, None, "~ is not expanded by the server");
            app.set_default_dir("/home/me/proj", cx);
            assert_eq!(app.prefs.default_dir.as_deref(), Some("/home/me/proj"));
            app.set_default_dir("", cx);
            assert_eq!(app.new_session_dir(None, None, None), None);
        });
    }

    #[gpui::test]
    fn settings_controls_set_use_current_and_reset(cx: &mut TestAppContext) {
        let (view, cx) = mount(cx);
        view.update(cx, |app, cx| {
            app.account = Some(account());
            app.session.working_dir = Some("/home/jeremy/jcode-mobile".into());
            app.screen = Screen::Settings;
            cx.notify();
        });
        settle_settings_fixture(&view, cx);
        tap(cx, "default-dir-current");
        view.read_with(cx, |app, _| assert_eq!(app.prefs.default_dir.as_deref(), Some("/home/jeremy/jcode-mobile")));
        draw(cx);
        assert!(cx.debug_bounds("default-dir-current").is_none(), "hidden once current is the default");
        tap(cx, "default-dir-reset");
        view.read_with(cx, |app, _| assert_eq!(app.prefs.default_dir, None));
        draw(cx);
        assert!(cx.debug_bounds("default-dir-reset").is_none(), "reset hidden when unset");
        tap(cx, "default-dir-input");
        cx.simulate_input("/srv/app");
        tap(cx, "default-dir-save");
        view.read_with(cx, |app, cx| {
            assert_eq!(app.prefs.default_dir.as_deref(), Some("/srv/app"));
            assert_eq!(app.default_dir_input.read(cx).text(), "/srv/app");
        });
        draw(cx);
        assert!(cx.debug_bounds("default-dir-save").is_none(), "save hidden once saved");
    }
}
