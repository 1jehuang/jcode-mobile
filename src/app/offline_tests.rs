//! Safety contract for repeatable offline interaction campaigns.
use super::*;
use gpui::TestAppContext;

#[gpui::test]
fn pull_reconnect_owns_status_until_indicator_settles(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx)
    });
    view.update(cx, |app, _| {
        assert!(!app.feel_reconnect_visible());
        app.feel.g.pull = crate::gestures::PULL_TRIGGER;
        assert!(app.feel_reconnect_visible(), "pull preview must suppress duplicate status");
        app.feel.g.refreshing = Some(std::time::Instant::now());
        app.feel.g.pull = 0.;
        assert!(app.feel_reconnect_visible(), "refresh owns status even before layout catches up");
        app.feel.g.refreshing = None;
        app.feel.g.pull = 12.;
        assert!(app.feel_reconnect_visible(), "settling indicator still owns status");
        app.feel.g.pull = 0.;
        assert!(!app.feel_reconnect_visible(), "normal status returns once settled");
    });
}

#[gpui::test]
fn fuzz_instances_block_external_actions_and_preserve_stored_credentials(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        MobileApp::new_offline(SafeArea { top: 20., bottom: 0. }, window, cx)
    });
    let directory = std::env::temp_dir().join(format!(
        "jcode-mobile-offline-safety-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    ));
    let path = directory.join("fixture-account.json");
    let account = Account {
        api_key: "offline-fixture-not-a-real-key".into(),
        api_base: "http://127.0.0.1:9".into(),
        email: Some("offline@example.test".into()),
    };
    AccountStore::new(path.clone()).save(&account).unwrap();
    let before = std::fs::read(&path).unwrap();
    view.update(cx, |app, cx| {
        assert!(app.offline);
        assert!(app.account.is_none(), "constructor loaded a real account");
        app.store = AccountStore::new(path.clone());
        app.email.update(cx, |input, cx| input.set_text("offline@example.test", cx));
        app.code.update(cx, |input, cx| input.set_text("123456", cx));
        app.sign_in_step(cx);
        app.google_sign_in(cx);
        assert!(!app.signing_in && app.browser_login.is_none());
        app.connect(account.clone(), cx);
        assert!(app.account.is_none(), "offline connect changed account");
        // A synthetic signed-in account exercises the same sidebar/tab actions
        // while the transport boundary remains disabled.
        app.account = Some(account.clone());
        app.push_token = Some("fixture-push-token".into());
        app.start_connection(None, cx);
        app.register_push(cx);
        app.unregister_push(cx);
        app.refresh_synced_sessions(cx);
        app.continue_synced_in_cloud(cx);
        app.delete_account_tap(cx);
        app.delete_account_tap(cx);
        assert_eq!(app.delete_step, 0);
        assert!(app.connection.is_none());
        app.send(|_| panic!("offline transport evaluated an outgoing request"));
        app.sign_out(cx);
        assert!(app.account.is_none());
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read(&path).unwrap(), before, "fuzz sign-out changed account file");
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
