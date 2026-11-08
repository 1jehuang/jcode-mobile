//! Glue between `ios_ext` and the app: deep links open sessions, and the
//! Live Activity mirrors whichever session is busy.

use futures::StreamExt;
use gpui::Context;

use super::MobileApp;
use crate::ios_ext::{self, DeepLink};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{init, lifecycle_coverage::fixture_app};
    use gpui::TestAppContext;

    #[test]
    fn offline_parser_fixtures_do_not_install_global_url_channels() {
        for (url, expected) in [
            ("jcode://new/", Some(DeepLink::NewSession { prompt: None })),
            (
                "jcode://ask?text=%20hello+%E2%9C%93%20&unused=x",
                Some(DeepLink::NewSession {
                    prompt: Some("hello ✓".into()),
                }),
            ),
            (
                "jcode://new?text=%20%20",
                Some(DeepLink::NewSession { prompt: None }),
            ),
            (
                "jcode://new?text=literal%ZZ%",
                Some(DeepLink::NewSession {
                    prompt: Some("literal%ZZ%".into()),
                }),
            ),
            (
                "jcode://session/fixture%2Done/",
                Some(DeepLink::OpenSession {
                    session_id: "fixture-one".into(),
                }),
            ),
            ("jcode://session/", None),
            ("https://fixture.invalid/session/one", None),
            ("jcode://unknown", None),
        ] {
            assert_eq!(ios_ext::parse(url), expected, "{url}");
        }
    }

    #[gpui::test]
    fn cold_signed_out_links_never_queue_prompts_or_create_tabs(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(fixture_app);
        view.update(cx, |app, cx| {
            app.sign_out(cx);
            let tab = app.active_tab;
            for url in [
                "jcode://new?text=not-authorized",
                "jcode://session/unknown",
                "jcode://new",
            ] {
                app.on_deep_link(ios_ext::parse(url).unwrap(), cx);
                assert_eq!(app.active_tab, tab);
                assert_eq!(app.tab_order, vec![tab]);
                assert!(app.auto_send.is_none());
                assert!(app.connection.is_none());
            }
        });
    }

    #[gpui::test]
    fn warm_links_reuse_active_and_parked_sessions_and_replace_pending_prompt(
        cx: &mut TestAppContext,
    ) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(fixture_app);
        view.update(cx, |app, cx| {
            let first = app.active_tab;
            app.session.session_id = Some("fixture-one".into());
            app.composer
                .update(cx, |input, cx| input.set_text("saved original", cx));
            app.on_deep_link(ios_ext::parse("jcode://session/fixture-one").unwrap(), cx);
            assert_eq!(app.active_tab, first);
            assert!(app.parked.is_empty());
            app.on_deep_link(
                ios_ext::parse("jcode://ask?text=queued+prompt").unwrap(),
                cx,
            );
            let second = app.active_tab;
            assert_ne!(second, first);
            assert_eq!(app.auto_send.as_deref(), Some("queued prompt"));
            assert!(app.composer.read(cx).text().is_empty());
            assert!(
                app.connection.is_none(),
                "offline dispatch never starts a connection"
            );
            app.on_deep_link(ios_ext::parse("jcode://new").unwrap(), cx);
            assert!(
                app.auto_send.is_none(),
                "plain new must not inherit a previous prompt"
            );
            app.on_deep_link(ios_ext::parse("jcode://session/fixture-one").unwrap(), cx);
            assert_eq!(app.active_tab, first);
            assert_eq!(app.composer.read(cx).text(), "saved original");
            let count = app.tab_order.len();
            app.on_deep_link(ios_ext::parse("jcode://session/fixture-one").unwrap(), cx);
            assert_eq!(
                app.tab_order.len(),
                count,
                "warm repeated links must not duplicate tabs"
            );
            app.on_deep_link(ios_ext::parse("jcode://session/not-open").unwrap(), cx);
            assert_eq!(app.tab_order.len(), count + 1);
            assert!(app.connection.is_none());
        });
    }
}

impl MobileApp {
    pub(super) fn install_deep_links(&mut self, cx: &mut Context<Self>) {
        let mut rx = ios_ext::install();
        cx.spawn(async move |this, cx| {
            while let Some(link) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.on_deep_link(link, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_deep_link(&mut self, link: DeepLink, cx: &mut Context<Self>) {
        log::info!("deep link: {link:?}");
        match link {
            DeepLink::OpenSession { session_id } => self.open_session(session_id, None, cx),
            DeepLink::NewSession { prompt } => {
                if self.account.is_none() {
                    return;
                }
                self.new_tab(cx);
                // A plain new-session link must not inherit a previous prompt.
                // pump sends it once the new session connects.
                self.auto_send = prompt;
            }
        }
    }

    /// Show every agent on the account in the single Live Activity: the
    /// Worker's account-wide view, corrected by this app's open tabs.
    pub(super) fn sync_live_activity(&self) {
        let local: Vec<_> = std::iter::once(&self.session)
            .chain(self.parked.iter().map(|p| &p.session))
            .filter_map(ios_ext::local_agent)
            .collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        let merged = jcode_mobile_core::agents::merge(self.agents_status.as_ref(), &local, now);
        ios_ext::sync_live(&merged);
    }
}
