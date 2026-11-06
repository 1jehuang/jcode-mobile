//! Read-only neighboring session peek. Never mounts an editor or mutates the
//! parked session's scroll position, draft, focus, or tool expansion state.
use super::*;

impl MobileApp {
    pub(super) fn render_panel_preview(&self, id: u64, t: &Theme) -> Option<AnyElement> {
        let session = self.tab_session(id)?;
        let title = session.session_title.clone().unwrap_or_else(|| "New session".into());
        let mut rows = Vec::new();
        // Bound work during gesture frames, retaining transcript order.
        for entry in session.transcript.iter().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
            let user = entry.role == Role::User;
            let text = if entry.text.is_empty() {
                if !entry.tool_calls.is_empty() { "Using tools…".to_owned() }
                else if !entry.reasoning.is_empty() { "Thinking…".to_owned() }
                else { continue }
            } else {
                entry.text.chars().take(1600).collect()
            };
            rows.push(div().w_full().min_w_0().px(px(10.)).py(px(8.))
                .rounded(px(8.))
                .bg(if user { Theme::prompt_background(0) } else { t.background })
                .child(label(if user { "You" } else { "Jcode" }, t.text_muted, 10.))
                .child(label(text, t.text, 13.).line_height(crate::system::fs(20.))));
        }
        Some(div().size_full().flex().flex_col().px(px(14.)).py(px(12.)).gap(px(12.))
            .child(label(title, t.text_strong, 15.).truncate().flex_none())
            .child(div().flex_1().min_h_0().overflow_hidden().flex().flex_col().justify_end().gap(px(12.))
                .when(rows.is_empty(), |d| d.child(label("What would you like to work on?", t.text, 22.)))
                .children(rows))
            .child(label("Session preview", t.text_muted, 11.).flex_none())
            .into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn neighbor_preview_preserves_active_and_parked_drafts(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx)
        });
        view.update(cx, |app, cx| {
            let first = app.active_tab;
            app.session.session_title = Some("Neighbor session".into());
            app.composer.update(cx, |input, cx| input.set_text("Neighbor draft", cx));
            app.park_active(cx);
            let second = app.next_tab;
            app.next_tab += 1;
            app.active_tab = second;
            app.tab_order.push(second);
            app.composer.update(cx, |input, cx| input.set_text("Current draft", cx));
            for _ in 0..20 {
                assert!(app.render_panel_preview(first, &Theme::pure_black()).is_some());
                assert!(app.render_panel_preview(u64::MAX, &Theme::pure_black()).is_none());
            }
            assert_eq!(app.active_tab, second);
            assert_eq!(app.composer.read(cx).text(), "Current draft");
            assert_eq!(app.parked.len(), 1);
            app.focus_tab(first, cx);
            assert_eq!(app.composer.read(cx).text(), "Neighbor draft");
            assert_eq!(app.session.session_title.as_deref(), Some("Neighbor session"));
        });
    }
}
