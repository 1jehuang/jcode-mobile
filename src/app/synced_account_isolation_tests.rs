//! Same-credential reauthentication must not revive a previous login's requests.
//! Actual production completion handlers are injected, never HTTP or SSH.
use super::super::account_coverage::{account, draw, mount};
use super::*;
use gpui::TestAppContext;

fn command() -> Command {
    serde_json::from_value(serde_json::json!({
        "command_id": "same-poll", "kind": "transfer", "session_id": "local-work",
        "status": "done", "result_session_id": "old-login-destination"
    }))
    .unwrap()
}

#[gpui::test]
fn external_lookup_origin_rejects_manual_tab_or_session_navigation(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.adopt_account(account(), cx);
        app.session.session_id = Some("origin-session".into());
        let tab = app.active_tab;
        let session = app.session.session_id.clone();
        assert!(app.external_lookup_origin_current(tab, &session));
        app.new_tab_in_dir(None, cx);
        assert!(!app.external_lookup_origin_current(tab, &session));
        app.focus_tab(tab, cx);
        app.session.session_id = Some("manually-opened".into());
        assert!(!app.external_lookup_origin_current(tab, &session));
    });
}

#[gpui::test]
fn uncertain_submission_needs_positive_reconciliation(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let expected = account();
        app.adopt_account(expected.clone(), cx);
        app.synced = State::fixture();
        app.synced.uncertain_submission = true;
        app.synced.submitted_draft = Some(("local-work".into(), "draft".into()));
        app.complete_synced_refresh(&expected, app.auth_generation, Ok((vec![], vec![])), cx);
        assert!(app.synced.uncertain_submission);
        app.complete_synced_refresh(&expected, app.auth_generation, Err("offline".into()), cx);
        assert!(app.synced.uncertain_submission);
        app.complete_synced_refresh(&expected, app.auth_generation, Ok((vec![], vec![command()])), cx);
        assert!(app.synced.uncertain_submission, "a same-source command from another client cannot identify our lost POST");
        app.synced.uncertain_command = Some(("same-poll".into(), "local-work".into()));
        app.complete_synced_refresh(&expected, app.auth_generation, Ok((vec![], vec![command()])), cx);
        assert!(!app.synced.uncertain_submission);
    });
}

#[gpui::test]
fn invalid_poll_stays_blocked_until_valid_matching_refresh(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let expected = account();
        app.adopt_account(expected.clone(), cx);
        for status in ["unknown", "done"] {
            app.synced = State::fixture();
            app.synced.polling = Some("same-poll".into());
            let mut invalid = command();
            invalid.status = status.into();
            invalid.result_session_id = None;
            assert!(app.complete_transfer_poll(&expected, app.auth_generation, "same-poll", "local-work", Ok(invalid), cx));
            assert!(app.synced.uncertain_submission);
            app.complete_synced_refresh(&expected, app.auth_generation, Ok((vec![], vec![])), cx);
            assert!(app.synced.uncertain_submission);
            app.complete_synced_refresh(&expected, app.auth_generation, Ok((vec![], vec![command()])), cx);
            assert!(!app.synced.uncertain_submission);
        }
    });
}

#[gpui::test]
fn same_account_aba_refresh_cannot_replace_new_login_inventory(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let expected = account();
        app.adopt_account(expected.clone(), cx);
        let old = app.auth_generation;
        app.sign_out(cx);
        app.adopt_account(expected.clone(), cx);
        app.synced = State::fixture();
        app.synced.loading = true;
        app.synced.uncertain_submission = true;
        app.complete_synced_refresh(&expected, old, Ok((vec![], vec![])), cx);
        assert!(
            app.synced.loading,
            "old refresh cleared replacement login's pending request"
        );
        assert!(
            !app.synced.sessions.is_empty(),
            "old refresh replaced new login inventory"
        );
        assert!(app.synced.uncertain_submission);
        app.complete_synced_refresh(&expected, old, Err("old refresh error".into()), cx);
        assert!(app.synced.error.is_none());
        app.complete_synced_refresh(&expected, app.auth_generation, Ok((vec![], vec![])), cx);
        assert!(!app.synced.loading && app.synced.uncertain_submission);
        assert!(app.synced.sessions.is_empty());
    });
    draw(cx);
}

#[gpui::test]
fn same_account_aba_transfer_cannot_clear_new_login_draft_or_install_old_command(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let expected = account();
        app.adopt_account(expected.clone(), cx);
        let old = app.auth_generation;
        app.sign_out(cx);
        app.adopt_account(expected.clone(), cx);
        app.synced = State::fixture();
        app.synced.submitting = true;
        app.synced.submitted_draft = Some(("local-work".into(), "new login continuation draft".into()));
        app.transfer_input.update(cx, |input, cx| {
            input.set_text("new login continuation draft", cx)
        });
        app.complete_synced_transfer(&expected, old, Ok(command()), cx);
        assert_eq!(
            app.transfer_input.read(cx).text(),
            "new login continuation draft",
            "old transfer erased new login draft"
        );
        assert!(app.synced.submitting);
        assert!(
            !app.synced
                .commands
                .iter()
                .any(|c| c.command_id == "same-poll")
        );
        let (_, feedback) = haptics::capture(|| {
            app.complete_synced_transfer(&expected, old, Err("old transfer failure".into()), cx);
        });
        assert!(feedback.is_empty());
        assert!(!app.synced.uncertain_submission && app.synced.error.is_none());
        app.complete_synced_transfer(&expected, app.auth_generation, Ok(command()), cx);
        assert!(!app.synced.submitting);
        assert!(app.transfer_input.read(cx).text().is_empty());
        assert!(
            app.synced
                .commands
                .iter()
                .any(|c| c.command_id == "same-poll")
        );
    });
    draw(cx);
}

#[gpui::test]
fn transfer_response_preserves_edited_or_other_session_drafts(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let expected = account();
        app.adopt_account(expected.clone(), cx);
        for (selected, draft) in [("local-work", "edited draft"), ("other-local", "original draft")] {
            app.synced = State::fixture();
            app.synced.submitting = true;
            app.synced.selected = Some(selected.into());
            app.synced.submitted_draft = Some(("local-work".into(), "original draft".into()));
            app.transfer_input.update(cx, |input, cx| input.set_text(draft, cx));
            app.complete_synced_transfer(&expected, app.auth_generation, Ok(command()), cx);
            assert_eq!(app.transfer_input.read(cx).text(), draft);
            assert!(!app.synced.submitting);
        }
    });
}

#[gpui::test]
fn transfer_poll_rejects_unrelated_response_and_never_hijacks_another_tab(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let expected = account();
        app.adopt_account(expected.clone(), cx);
        for mismatch in ["id", "kind", "source"] {
            app.synced = State::fixture();
            app.synced.polling = Some("same-poll".into());
            let mut response = command();
            match mismatch {
                "id" => response.command_id = "wrong-id".into(),
                "kind" => response.kind = "message".into(),
                _ => response.session_id = Some("other-source".into()),
            }
            assert!(app.complete_transfer_poll(&expected, app.auth_generation, "same-poll", "local-work", Ok(response), cx));
            assert!(app.synced.polling.is_none());
            assert!(app.synced.error.is_some());
            assert!(!app.synced.commands.iter().any(|c| c.command_id == "same-poll" || c.command_id == "wrong-id"));
        }
        app.synced = State::fixture();
        app.synced.polling = Some("same-poll".into());
        app.drawer_open = true;
        app.session.session_id = Some("unrelated-cloud-tab".into());
        let tab = app.active_tab;
        assert!(app.complete_transfer_poll(&expected, app.auth_generation, "same-poll", "local-work", Ok(command()), cx));
        assert_eq!(app.active_tab, tab);
        assert_eq!(app.session.session_id.as_deref(), Some("unrelated-cloud-tab"));
    });
}

#[gpui::test]
fn same_account_aba_poll_cannot_finish_or_navigate_replacement_login(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        let expected = account();
        app.adopt_account(expected.clone(), cx);
        let old = app.auth_generation;
        app.sign_out(cx);
        app.adopt_account(expected.clone(), cx);
        app.synced = State::fixture();
        // Reusing the very same command id must not revive the previous loop.
        app.synced.polling = Some("same-poll".into());
        app.drawer_open = true;
        let tab = app.active_tab;
        let (_, feedback) = haptics::capture(|| {
            assert!(app.complete_transfer_poll(
                &expected,
                old,
                "same-poll",
                "local-work",
                Ok(command()),
                cx
            ));
        });
        assert_eq!(
            app.active_tab, tab,
            "old poll navigated to previous login's destination"
        );
        assert_eq!(app.synced.polling.as_deref(), Some("same-poll"));
        assert_eq!(app.synced.selected.as_deref(), Some("local-work"));
        assert!(feedback.is_empty());
        assert!(app.complete_transfer_poll(
            &expected,
            old,
            "same-poll",
            "local-work",
            Err("late poll failure".into()),
            cx
        ));
        assert!(app.synced.error.is_none());
        app.drawer_open = false;
        assert!(app.complete_transfer_poll(
            &expected,
            app.auth_generation,
            "retired-poll",
            "local-work",
            Ok(command()),
            cx
        ));
        assert_eq!(app.synced.polling.as_deref(), Some("same-poll"));
        assert!(app.complete_transfer_poll(
            &expected,
            app.auth_generation,
            "same-poll",
            "local-work",
            Ok(command()),
            cx
        ));
        assert!(app.synced.polling.is_none());
        assert!(
            app.synced
                .commands
                .iter()
                .any(|c| c.command_id == "same-poll")
        );
    });
    draw(cx);
}
