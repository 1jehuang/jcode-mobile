//! Background profile hydration for saved accounts. Names are display metadata,
//! never account identity, and refreshing them must not restart a conversation.
use super::*;

pub(super) fn same_account(current: Option<&Account>, expected: &Account) -> bool {
    current.is_some_and(|a| a.api_key == expected.api_key && a.api_base == expected.api_base)
}

impl MobileApp {
    pub(super) fn ensure_account_profile(&mut self, cx: &mut Context<Self>) {
        if self.offline || self.account_profile_requested {
            return;
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        self.account_profile_requested = true;
        let generation = self.auth_generation;
        let expected = account.clone();
        let task = cx.background_spawn(async move { cloud::refresh_account_profile(&account) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if let Ok(profile) = result {
                    this.complete_account_profile(&expected, generation, profile, cx);
                }
                // Missing profile support or an offline control plane must never
                // interrupt sign-in or an already connected session.
            })
            .ok();
        })
        .detach();
    }

    fn complete_account_profile(
        &mut self,
        expected: &Account,
        generation: u64,
        profile: Account,
        cx: &mut Context<Self>,
    ) {
        if generation != self.auth_generation || !same_account(self.account.as_ref(), expected) {
            return;
        }
        let Some(current) = self.account.as_mut() else {
            return;
        };
        if current.given_name == profile.given_name && current.name == profile.name {
            return;
        }
        // Do not overwrite credentials or email if a concurrent account-link
        // operation changed them while the profile request was in flight.
        current.given_name = profile.given_name;
        current.name = profile.name;
        if !self.offline {
            let _ = self.store.save(current);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn profile_hydration_updates_names_without_changing_session_or_email(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = super::super::navigation_coverage::mount(cx);
        view.update(cx, |app, cx| {
            let expected = app.account.clone().unwrap();
            let mut profile = expected.clone();
            profile.given_name = Some("Jeremy".into());
            profile.name = Some("Jeremy Huang".into());
            app.session.session_id = Some("keep-this-conversation".into());
            app.account.as_mut().unwrap().email = Some("linked@example.test".into());
            let (_, feedback) = crate::haptics::capture(|| {
                app.complete_account_profile(&expected, app.auth_generation, profile, cx);
            });
            assert!(feedback.is_empty());
            let account = app.account.as_ref().unwrap();
            assert_eq!(account.given_name.as_deref(), Some("Jeremy"));
            assert_eq!(account.email.as_deref(), Some("linked@example.test"));
            assert_eq!(
                app.session.session_id.as_deref(),
                Some("keep-this-conversation")
            );
            assert!(same_account(app.account.as_ref(), &expected));
        });
    }

    #[gpui::test]
    fn stale_profile_and_offline_fetch_cannot_affect_another_login(cx: &mut gpui::TestAppContext) {
        let (view, cx) = super::super::navigation_coverage::mount(cx);
        view.update(cx, |app, cx| {
            app.ensure_account_profile(cx);
            assert!(
                !app.account_profile_requested,
                "offline fixtures must never fetch"
            );
            let expected = app.account.clone().unwrap();
            let mut profile = expected.clone();
            profile.given_name = Some("Wrong user".into());
            app.complete_account_profile(
                &expected,
                app.auth_generation.wrapping_add(1),
                profile.clone(),
                cx,
            );
            assert!(app.account.as_ref().unwrap().given_name.is_none());
            app.account.as_mut().unwrap().api_key = "different-login".into();
            app.complete_account_profile(&expected, app.auth_generation, profile.clone(), cx);
            assert!(app.account.as_ref().unwrap().given_name.is_none());
            app.account = None;
            app.complete_account_profile(&expected, app.auth_generation, profile, cx);
            assert!(app.account.is_none());
        });
    }
}
