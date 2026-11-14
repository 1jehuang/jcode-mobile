use super::*;

#[gpui::test]
fn old_connection_events_cannot_repopulate_signed_out_or_replaced_account(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    let mut senders = Vec::new();
    view.update(cx, |app, cx| {
        app.adopt_account(account(), cx);
        for _ in 0..2 {
            let (tx, rx) = unbounded();
            app.pump(app.active_tab, rx, cx);
            senders.push(tx);
        }
        app.sign_out(cx);
    });
    let delta = || {
        ConnectionOutput::Event(jcode_mobile_core::ApiEvent::TextDelta {
            session_id: "old-account-session".into(),
            message_id: None,
            text: "private late transcript from old account".into(),
        })
    };
    senders[0].unbounded_send(delta()).unwrap();
    cx.run_until_parked();
    draw(cx);
    view.update(cx, |app, cx| {
        assert!(
            app.session.transcript.is_empty(),
            "old socket repopulated signed-out state"
        );
        let mut next = account();
        next.api_key = "replacement-account".into();
        app.adopt_account(next, cx);
    });
    senders[1].unbounded_send(delta()).unwrap();
    cx.run_until_parked();
    draw(cx);
    let current = view.update(cx, |app, cx| {
        assert!(
            app.session.transcript.is_empty(),
            "old socket crossed account boundary"
        );
        let (tx, rx) = unbounded();
        app.pump(app.active_tab, rx, cx);
        tx
    });
    current
        .unbounded_send(ConnectionOutput::Event(
            jcode_mobile_core::ApiEvent::TextDelta {
                session_id: "new-account-session".into(),
                message_id: None,
                text: "current account transcript".into(),
            },
        ))
        .unwrap();
    cx.run_until_parked();
    draw(cx);
    view.read_with(cx, |app, _| {
        assert_eq!(app.session.transcript.len(), 1);
        assert_eq!(app.session.transcript[0].text, "current account transcript");
    });
}

#[gpui::test]
fn adopting_another_account_discards_old_workspace_before_drawing(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    let old_tab = view.update(cx, |app, cx| {
        app.adopt_account(account(), cx);
        app.session.transcript.push(TranscriptEntry {
            id: 1,
            role: Role::User,
            text: "private first-account conversation".into(),
            reasoning: String::new(),
            tool_calls: vec![],
            is_streaming: false,
            is_queued: false,
        });
        let old = app.active_tab;
        app.new_tab(cx);
        app.composer.update(cx, |input, cx| {
            input.set_text("private first-account draft", cx)
        });
        app.auto_send = Some("private first-account external prompt".into());
        app.drawer_search
            .update(cx, |input, cx| input.set_text("private search", cx));
        app.dir_input
            .update(cx, |input, cx| input.set_text("/private/old-account", cx));
        app.model_picker = true;
        app.dir_picker = true;
        app.drawer_open = true;
        app.drawer_t = 1.;
        app.tab_menu = Some(old);
        app.delete_step = 1;
        app.delete_error = Some("private deletion error".into());
        let mut next = account();
        next.api_key = "different-account-key".into();
        let generation = app.begin_auth();
        app.complete_sign_in(
            generation,
            Ok(SignInOutcome::Authenticated(next.clone())),
            cx,
        );
        assert_eq!(app.account, Some(next));
        old
    });
    draw(cx);
    view.update(cx, |app, cx| {
        assert!(
            app.parked.is_empty(),
            "old account conversations remain accessible"
        );
        assert_eq!(app.tab_order, [app.active_tab]);
        assert!(app.session.transcript.is_empty());
        assert!(
            app.composer.read(cx).text().is_empty(),
            "old draft crossed accounts"
        );
        assert!(
            app.auto_send.is_none(),
            "old prompt could dispatch under new credentials"
        );
        assert!(app.drawer_search.read(cx).text().is_empty());
        assert!(app.dir_input.read(cx).text().is_empty());
        assert!(!app.model_picker && !app.dir_picker && !app.drawer_open);
        assert!(app.tab_menu.is_none());
        assert_eq!(app.delete_step, 0);
        assert!(app.delete_error.is_none());
        app.focus_tab(old_tab, cx);
        assert!(
            app.session.transcript.is_empty(),
            "old tab can still be restored"
        );
    });
    draw(cx);
}

#[gpui::test]
fn drawn_signout_and_relogin_do_not_reopen_previous_account_map(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.adopt_account(account(), cx);
        app.map.show_for_test();
        app.screen = Screen::Settings;
        cx.notify();
    });
    settle_settings_fixture(&view, cx);
    tap(cx, "sign-out");
    view.update(cx, |app, cx| {
        assert!(app.account.is_none());
        assert!(
            !app.map.visible(),
            "logout retained old account's session map"
        );
        app.adopt_account(account(), cx);
    });
    draw(cx);
    view.read_with(cx, |app, _| assert!(!app.map.visible()));
}
