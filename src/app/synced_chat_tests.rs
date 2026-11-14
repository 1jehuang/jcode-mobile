use super::*;
use crate::app::{self, lifecycle_coverage::fixture_app};

#[path = "synced_chat_more_tests.rs"]
mod more_tests;

fn live() -> SyncedSession {
    serde_json::from_value(serde_json::json!({"session_id":"local-chat","source":"local","title":"Computer task","live":true,"busy":false})).unwrap()
}

fn snapshot(live: bool, busy: bool, answer: &str) -> Result<SyncedTranscript, String> {
    let mut session = self::live();
    session.live = live;
    session.busy = busy;
    Ok(SyncedTranscript {
        session,
        messages: serde_json::from_value(serde_json::json!([
            {"seq":0,"role":"user","text":"Question"},
            {"seq":3,"role":"assistant","text":answer},
            {"seq":8,"role":"tool","text":"Tool output"}
        ]))
        .unwrap(),
    })
}

fn command(status: &str) -> Command {
    serde_json::from_value(serde_json::json!({"command_id":"12345678-1234-1234-1234-123456789abc","kind":"message","session_id":"local-chat","status":status,"error": if status == "failed" { Some("Desktop rejected message") } else { None }})).unwrap()
}

#[gpui::test]
fn opening_live_local_creates_chat_without_cloud_and_preserves_other_draft(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        app.composer
            .update(cx, |input, cx| input.set_text("Cloud draft", cx));
        let cloud = app.active_tab;
        app.open_local_chat(live(), Some("Notification draft".into()), cx);
        let local = app.active_tab;
        assert_ne!(local, cloud);
        assert!(app.is_local_chat());
        assert!(app.connection.is_none());
        assert_eq!(app.session.session_id.as_deref(), Some("local-chat"));
        assert_eq!(app.composer.read(cx).text(), "Notification draft");
        assert!(app.auto_send.is_none());
        assert!(!app.drawer_open);
        app.open_local_chat(live(), Some("Must not replace".into()), cx);
        assert_eq!(app.tab_order.len(), 2);
        assert_eq!(app.composer.read(cx).text(), "Notification draft");
        app.focus_tab(cloud, cx);
        assert_eq!(app.composer.read(cx).text(), "Cloud draft");
        app.focus_tab(local, cx);
        assert_eq!(app.composer.read(cx).text(), "Notification draft");
        app.reconnect(cx);
        assert!(app.connection.is_none());
        app.offline = false;
        app.reconnect(cx);
        app.start_connection(Some("local-chat".into()), cx);
        app.fork_tab(local, cx);
        app.feel_rename_tab(local, cx);
        assert!(app.connection.is_none());
        assert_eq!(app.tab_order.len(), 2);
        app.offline = true;
    });
}

#[gpui::test]
fn polling_updates_parked_tab_replaces_rows_and_tracks_busy_presence(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        let cloud = app.active_tab;
        app.open_local_chat(live(), None, cx);
        let local = app.active_tab;
        app.apply_local_poll(local, snapshot(true, true, "First answer"), None);
        assert!(app.session.is_processing);
        assert!(app.local_chat_ready(&app.session));
        assert_eq!(app.session.transcript.len(), 3);
        assert_eq!(app.session.transcript[2].role, Role::System);
        assert!(app.session.transcript[2].text.contains("Tool output"));
        app.focus_tab(cloud, cx);
        app.apply_local_poll(local, snapshot(true, false, "Rewritten answer"), None);
        assert!(app.session.transcript.is_empty());
        app.focus_tab(local, cx);
        assert_eq!(app.session.transcript.len(), 3);
        assert_eq!(app.session.transcript[1].text, "Rewritten answer");
        assert!(!app.session.is_processing);
        app.apply_local_poll(local, snapshot(false, false, "Rewritten answer"), None);
        assert!(!app.local_chat_ready(&app.session));
        assert_eq!(app.session.phase, ConnectionPhase::Disconnected);
        assert!(
            app.session
                .error_banner
                .as_ref()
                .unwrap()
                .contains("offline")
        );
        app.apply_local_poll(local, snapshot(true, false, "Rewritten answer"), None);
        assert!(app.local_chat_ready(&app.session));
        assert!(app.session.error_banner.is_none());
    });
}

#[gpui::test]
fn message_status_never_navigates_and_failures_do_not_retry(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        app.open_local_chat(live(), None, cx);
        let local = app.active_tab;
        app.synced.chats.get_mut(&local).unwrap().pending = Some(command("queued").command_id);
        for status in ["queued", "running"] {
            app.apply_local_poll(
                local,
                snapshot(true, true, "Working"),
                Some(Ok(command(status))),
            );
            assert!(!app.local_chat_ready(&app.session));
            assert_eq!(app.active_tab, local);
        }
        app.apply_local_poll(
            local,
            snapshot(true, false, "Done"),
            Some(Ok(command("done"))),
        );
        assert!(app.local_chat_ready(&app.session));
        assert_eq!(app.session.session_id.as_deref(), Some("local-chat"));
        assert_eq!(app.tab_order.len(), 2);
        app.synced.chats.get_mut(&local).unwrap().pending = Some(command("queued").command_id);
        app.apply_local_poll(
            local,
            snapshot(true, true, "Done"),
            Some(Err("Network unavailable".into())),
        );
        assert!(app.synced.chats[&local].pending.is_some());
        app.apply_local_poll(
            local,
            snapshot(true, false, "Done"),
            Some(Ok(command("failed"))),
        );
        assert!(app.synced.chats[&local].pending.is_none());
        assert!(
            app.session
                .error_banner
                .as_ref()
                .unwrap()
                .contains("Desktop rejected")
        );
        app.apply_local_poll(local, snapshot(true, false, "Done"), None);
        assert!(
            app.session
                .error_banner
                .as_ref()
                .unwrap()
                .contains("Desktop rejected")
        );
        assert!(app.connection.is_none());
    });
}

#[gpui::test]
fn uncertain_submission_and_offline_preserve_draft_and_account_fences(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        app.open_local_chat(live(), None, cx);
        let local = app.active_tab;
        let account = app.account.clone().unwrap();
        let generation = app.auth_generation;
        assert!(app.local_chat_current(local, &account, generation));
        app.apply_local_poll(local, snapshot(true, false, "Existing"), None);
        app.composer
            .update(cx, |input, cx| input.set_text("Keep my message", cx));
        app.send_draft(cx);
        assert_eq!(app.composer.read(cx).text(), "Keep my message");
        assert!(!app.synced.chats[&local].submitting);
        app.synced.chats.get_mut(&local).unwrap().uncertain = true;
        app.apply_local_poll(local, snapshot(true, false, "Existing"), None);
        assert!(!app.local_chat_ready(&app.session));
        assert!(
            app.session
                .error_banner
                .as_ref()
                .unwrap()
                .contains("uncertain")
        );
        assert!(!app.local_chat_current(local, &account, generation.wrapping_add(1)));
        let mut other = account.clone();
        other.api_key = "other account".into();
        assert!(!app.local_chat_current(local, &other, generation));
        app.close_tab(local, cx);
        assert!(!app.local_chat_current(local, &account, generation));
    });
}

#[gpui::test]
fn local_composer_hides_cloud_controls_and_stop_button(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        app.open_local_chat(live(), None, cx);
        app.apply_local_poll(app.active_tab, snapshot(true, true, "Working"), None);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("local-chat-location").is_some());
    assert!(cx.debug_bounds("chat-metadata").is_none());
    assert!(cx.debug_bounds("stop").is_none());
    assert!(cx.debug_bounds("send").is_some());
}
