//! Stateful handler journeys complement raw touch fuzzing. Service outcomes are
//! injected, not sent. Unlike the raw campaign, state is not reset per action.
use super::account_coverage::{account, draw, mount};
use super::*;
use gpui::TestAppContext;
use std::collections::BTreeMap;

#[derive(Default)]
struct ExpectedTab {
    draft: String,
    pending: Option<String>,
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut x = *state;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

#[gpui::test]
fn seeded_stateful_account_tab_draft_and_pending_prompt_journeys(cx: &mut TestAppContext) {
    for initial in [0x4a4f55524e4559_u64, 42, 0, u64::MAX] {
        let (view, cx) = mount(cx);
        let mut random = initial;
        let mut model = BTreeMap::from([(0_u64, ExpectedTab::default())]);
        let mut order = vec![0_u64];
        let mut active = 0_u64;
        view.update(cx, |app, cx| app.adopt_account(account(), cx));
        for step in 0..256 {
            let action = next(&mut random) % 10;
            let choice = next(&mut random);
            cx.update(|window, cx| {
                view.update(cx, |app, cx| {
                    match action {
                        0 | 1 => {
                            let text = ["x", "👩🏽‍💻", "e\u{301}", "\n", "世界"][choice as usize % 5];
                            app.composer
                                .update(cx, |input, cx| input.sim_insert(text, window, cx));
                            model.get_mut(&active).unwrap().draft.push_str(text);
                        }
                        2 if order.len() < 8 => {
                            let new_id = app.next_tab;
                            let at = order.iter().position(|id| *id == active).unwrap() + 1;
                            app.new_tab(cx);
                            order.insert(at, new_id);
                            model.insert(new_id, ExpectedTab::default());
                            active = new_id;
                        }
                        3 => {
                            let target = order[choice as usize % order.len()];
                            app.focus_tab(target, cx);
                            active = target;
                        }
                        4 if order.len() > 1 => {
                            let position = choice as usize % order.len();
                            let closing = order.remove(position);
                            model.remove(&closing);
                            if closing == active {
                                active = order[position.min(order.len() - 1)];
                            }
                            app.close_tab(closing, cx);
                        }
                        5 => {
                            let prompt =
                                format!("queued for tab {active}, seed {initial}, step {step}");
                            app.auto_send = Some(prompt.clone());
                            model.get_mut(&active).unwrap().pending = Some(prompt);
                        }
                        6 => {
                            app.session.phase = ConnectionPhase::Connected;
                            app.session.session_id = Some(format!("fixture-{active}"));
                            let had_text = !model[&active].draft.trim().is_empty();
                            app.send_draft(cx);
                            if had_text {
                                model.get_mut(&active).unwrap().draft.clear();
                            }
                        }
                        7 => {
                            let pending_auth = app.begin_auth();
                            app.sign_out(cx);
                            model.clear();
                            model.insert(active, ExpectedTab::default());
                            order = vec![active];
                            app.complete_sign_in(
                                pending_auth,
                                Ok(SignInOutcome::Authenticated(account())),
                                cx,
                            );
                            assert!(app.account.is_none(), "stale auth resurrected after logout");
                            let auth = app.begin_auth();
                            app.complete_sign_in(
                                auth,
                                Ok(SignInOutcome::Authenticated(account())),
                                cx,
                            );
                        }
                        8 => {
                            app.device = Some(
                                [(320., 568.), (390., 844.), (844., 390.), (1024., 768.)]
                                    [choice as usize % 4],
                            );
                            app.cancel(cx);
                        }
                        _ => {
                            let before = app.account.clone();
                            app.delete_account_tap(cx);
                            assert_eq!(
                                app.account, before,
                                "offline delete crossed service boundary"
                            );
                        }
                    }
                    assert_eq!(
                        app.active_tab, active,
                        "seed={initial} step={step} action={action}"
                    );
                    assert_eq!(
                        app.tab_order, order,
                        "seed={initial} step={step} action={action}"
                    );
                    assert_eq!(app.parked.len() + 1, model.len());
                    assert_eq!(
                        app.composer.read(cx).text(),
                        model[&active].draft,
                        "draft seed={initial} step={step} action={action}"
                    );
                    assert_eq!(
                        app.auto_send, model[&active].pending,
                        "pending prompt seed={initial} step={step} action={action}"
                    );
                    assert!(app.offline && app.connection.is_none());
                    assert!(app.parked.iter().all(|tab| tab.connection.is_none()));
                    cx.notify();
                })
            });
            // Draw between handlers to include responsive layout and frame hooks.
            draw(cx);
        }
        for (&id, expected) in &model {
            view.update(cx, |app, cx| {
                app.focus_tab(id, cx);
                assert_eq!(
                    app.composer.read(cx).text(),
                    expected.draft,
                    "final draft seed={initial} tab={id}"
                );
                assert_eq!(
                    app.auto_send, expected.pending,
                    "final prompt seed={initial} tab={id}"
                );
            });
        }
        eprintln!(
            "stateful journey complete seed={initial} actions=256 surviving_tabs={}",
            model.len()
        );
    }
}
