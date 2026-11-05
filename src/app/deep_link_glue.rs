//! Glue between `ios_ext` and the app: deep links open sessions, and the
//! Live Activity mirrors whichever session is busy.

use futures::StreamExt;
use gpui::Context;

use super::MobileApp;
use crate::ios_ext::{self, DeepLink};

impl MobileApp {
    pub(super) fn install_deep_links(&mut self, cx: &mut Context<Self>) {
        let mut rx = ios_ext::install();
        cx.spawn(async move |this, cx| {
            while let Some(link) = rx.next().await {
                if this.update(cx, |this, cx| this.on_deep_link(link, cx)).is_err() {
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
                self.new_tab(cx);
                if let Some(p) = prompt {
                    // pump sends it once the new session connects
                    self.auto_send = Some(p);
                }
            }
        }
    }

    /// Mirror the busy session (the visible one first) in the Live Activity.
    pub(super) fn sync_live_activity(&self) {
        let state = ios_ext::live_state(&self.session)
            .or_else(|| self.parked.iter().find_map(|p| ios_ext::live_state(&p.session)));
        ios_ext::sync_live(state);
    }
}
