use super::*;

#[gpui::test]
fn live_drawer_row_opens_chat_instead_of_cloud_transfer(cx: &mut gpui::TestAppContext) {
    use crate::app::navigation_coverage::{drawer, mount, tap};
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.synced.sessions = vec![live()];
        app.synced.selected = Some("local-chat".into());
        cx.notify();
    });
    drawer(&view, cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("transfer-local-chat").is_none());
    tap(cx, "drawer-local-session-local-chat");
    view.read_with(cx, |app, _| {
        assert!(app.is_local_chat());
        assert_eq!(app.session.session_id.as_deref(), Some("local-chat"));
        assert!(app.connection.is_none());
        assert!(!app.drawer_open);
        assert_eq!(app.tab_order.len(), 2);
    });
}
#[gpui::test]
fn submission_completion_preserves_new_typing_and_parked_drafts(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        let cloud = app.active_tab;
        app.open_local_chat(live(), None, cx);
        let local = app.active_tab;
        let account = app.account.clone().unwrap();
        let generation = app.auth_generation;
        app.apply_local_poll(local, snapshot(true, false, "Existing"), None);
        app.composer
            .update(cx, |input, cx| input.set_text("New typing", cx));
        app.complete_local_submission(
            local,
            &account,
            generation,
            "Submitted text",
            Ok(command("queued")),
            cx,
        );
        assert_eq!(app.composer.read(cx).text(), "New typing");
        assert!(app.synced.chats[&local].pending.is_some());
        app.synced.chats.get_mut(&local).unwrap().pending = None;
        app.focus_tab(cloud, cx);
        app.composer
            .update(cx, |input, cx| input.set_text("Cloud is untouched", cx));
        app.complete_local_submission(
            local,
            &account,
            generation,
            "New typing",
            Ok(command("queued")),
            cx,
        );
        assert_eq!(app.composer.read(cx).text(), "Cloud is untouched");
        app.focus_tab(local, cx);
        assert_eq!(app.composer.read(cx).text(), "");
        app.composer
            .update(cx, |input, cx| input.set_text("Keep on network error", cx));
        app.synced.chats.get_mut(&local).unwrap().pending = None;
        app.complete_local_submission(
            local,
            &account,
            generation,
            "Keep on network error",
            Err("response lost".into()),
            cx,
        );
        assert!(app.local_delivery_uncertain(&app.session));
        assert_eq!(app.composer.read(cx).text(), "Keep on network error");
        app.acknowledge_local_delivery(cx);
        assert!(!app.local_delivery_uncertain(&app.session));
        assert!(app.synced.chats[&local].pending.is_none());
        assert_eq!(app.composer.read(cx).text(), "Keep on network error");
        app.complete_local_submission(
            local,
            &account,
            generation.wrapping_add(1),
            "Keep on network error",
            Ok(command("queued")),
            cx,
        );
        assert!(app.synced.chats[&local].pending.is_none());
        assert_eq!(app.composer.read(cx).text(), "Keep on network error");
    });
}

#[gpui::test]
fn already_open_local_notification_reply_is_draft_not_ssh_message(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        app.open_local_chat(live(), None, cx);
        let local = app.active_tab;
        app.apply_local_poll(local, snapshot(true, false, "Existing"), None);
        app.open_external_session("local-chat".into(), Some("Reply from push".into()), cx);
        assert_eq!(app.active_tab, local);
        assert_eq!(app.tab_order.len(), 2);
        assert_eq!(app.session.transcript.len(), 3);
        assert_eq!(app.composer.read(cx).text(), "Reply from push");
        assert!(app.connection.is_none());
        assert!(app.auto_send.is_none());
    });
}
