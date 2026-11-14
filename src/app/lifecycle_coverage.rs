//! Deterministic lifecycle fixtures. All app instances remain offline unless a
//! test checks a pure eligibility predicate. No platform channels are installed.
use super::*;
use gpui::TestAppContext;

pub(super) fn fixture_account() -> Account {
    Account {
        given_name: None,
        name: None,
        api_key: "lifecycle-fixture-not-a-key".into(),
        api_base: "https://fixture.invalid".into(),
        email: None,
    }
}

pub(super) fn fixture_app(window: &mut Window, cx: &mut Context<MobileApp>) -> MobileApp {
    let mut app = MobileApp::new_offline(
        SafeArea {
            top: 20.,
            bottom: 0.,
        },
        window,
        cx,
    );
    app.screen = Screen::Chat;
    app.account = Some(fixture_account());
    app.device = Some((390., 844.));
    app.drawer_open = false;
    app.drawer_t = 0.;
    app.auto_send = None;
    app.composer.update(cx, |input, cx| input.set_text("", cx));
    app
}

#[gpui::test]
fn scene_voice_eligibility_excludes_every_modal_and_transient_state(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, _| {
        assert!(
            !app.scene_allows_voice(),
            "offline fixtures cannot capture audio"
        );
        // Predicate only, never invoke begin_voice or install platform handlers.
        app.offline = false;
        assert!(app.scene_allows_voice());
        app.drawer_open = true;
        assert!(!app.scene_allows_voice());
        app.drawer_open = false;
        app.git_info.open = true;
        assert!(!app.scene_allows_voice());
        app.git_info.open = false;
        app.model_picker = true;
        assert!(!app.scene_allows_voice());
        app.model_picker = false;
        app.dir_picker = true;
        assert!(!app.scene_allows_voice());
        app.dir_picker = false;
        app.tab_menu = Some(app.active_tab);
        assert!(!app.scene_allows_voice());
        app.tab_menu = None;
        app.voice.busy = true;
        assert!(!app.scene_allows_voice());
        app.voice.busy = false;
        app.voice.starting = true;
        assert!(
            !app.scene_allows_voice(),
            "startup is independently ineligible"
        );
        app.voice.starting = false;
        for screen in [Screen::Settings, Screen::SignIn] {
            app.screen = screen;
            assert!(!app.scene_allows_voice());
        }
        app.screen = Screen::Chat;
        app.account = None;
        assert!(!app.scene_allows_voice());
        app.offline = true;
    });
}

#[gpui::test]
fn tab_switch_and_signout_cancel_mocked_pending_voice(cx: &mut TestAppContext) {
    cx.update(init);
    let (view, cx) = cx.add_window_view(fixture_app);
    view.update(cx, |app, cx| {
        let original_tab = app.active_tab;
        app.composer
            .update(cx, |input, cx| input.set_text("original draft", cx));
        // Mock pending capture. No Recording exists and no microphone is used.
        app.voice.busy = true;
        app.voice.starting = true;
        app.voice.auto_send = true;
        app.new_tab(cx);
        assert_ne!(app.active_tab, original_tab);
        assert!(!app.voice.busy && !app.voice.starting && !app.voice.auto_send);
        app.composer
            .update(cx, |input, cx| input.set_text("second draft", cx));
        app.voice.busy = true; // Mock in-flight transcription.
        app.focus_tab(original_tab, cx);
        assert!(!app.voice.busy);
        assert_eq!(app.composer.read(cx).text(), "original draft");
        app.voice.busy = true;
        app.voice.starting = true;
        app.sign_out(cx);
        assert!(!app.voice.busy && !app.voice.starting);
        assert!(app.voice.recording.is_none());
        assert!(app.account.is_none() && app.parked.is_empty());
        assert!(app.composer.read(cx).text().is_empty());
        assert!(app.screen == Screen::SignIn);
        assert!(app.connection.is_none());
    });
}
