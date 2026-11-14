use super::*;
use jcode_mobile_core::SessionImage;

#[path = "synced_chat_photo_http_tests.rs"]
mod http_tests;

fn photo(byte: u8) -> SessionImage {
    SessionImage::new("image/jpeg", vec![byte])
}

#[gpui::test]
fn closed_uncertain_local_photo_is_restored_on_reopen(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        app.open_local_chat(live(), None, cx);
        let tab = app.active_tab;
        app.attach.pending.push(photo(1));
        let chat = app.synced.chats.get_mut(&tab).unwrap();
        chat.submitted_images = vec![photo(1)];
        chat.uncertain = true;
        app.close_tab(tab, cx);
        app.open_local_chat(live(), None, cx);
        assert_eq!(app.active_tab, tab);
        assert_eq!(app.attach.pending, vec![photo(1)]);
        assert!(app.local_delivery_uncertain(&app.session));
    });
}

#[gpui::test]
fn photo_acknowledgement_preserves_new_attachments_and_parked_tabs(cx: &mut gpui::TestAppContext) {
    cx.update(app::init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        let cloud = app.active_tab;
        app.open_local_chat(live(), None, cx);
        let local = app.active_tab;
        app.apply_local_poll(local, snapshot(true, false, "Existing"), None);
        let account = app.account.clone().unwrap();
        let generation = app.auth_generation;
        app.attach.pending = vec![photo(1), photo(2)];
        app.synced.chats.get_mut(&local).unwrap().submitted_images = vec![photo(1)];
        app.complete_local_submission(local, &account, generation, "", Ok(command("queued")), cx);
        assert_eq!(app.attach.pending, vec![photo(1), photo(2)]);
        app.focus_tab(cloud, cx);
        app.attach.pending.push(photo(3));
        app.apply_local_poll(
            local,
            snapshot(true, false, "Done"),
            Some(Ok(command("done"))),
        );
        assert_eq!(app.attach.pending, vec![photo(3)]);
        app.focus_tab(local, cx);
        assert_eq!(app.attach.pending, vec![photo(2)]);
        app.synced.chats.get_mut(&local).unwrap().submitted_images = vec![photo(2)];
        app.complete_local_submission(
            local,
            &account,
            generation,
            "",
            Err("lost response".into()),
            cx,
        );
        assert_eq!(app.attach.pending, vec![photo(2)]);
        app.acknowledge_local_delivery(cx);
        app.complete_local_submission(
            local,
            &account,
            generation.wrapping_add(1),
            "",
            Ok(command("done")),
            cx,
        );
        assert_eq!(app.attach.pending, vec![photo(2)]);
        app.complete_local_submission(local, &account, generation, "", Ok(command("queued")), cx);
        app.apply_local_poll(
            local,
            snapshot(true, false, "Failed"),
            Some(Ok(command("failed"))),
        );
        assert_eq!(app.attach.pending, vec![photo(2)]);
    });
}
