//! Read-only cloud inventory. No credential contents enter the UI or transcript.
use super::*;
use crate::haptics::{self, Haptic};
use gpui::svg;
use jcode_mobile_core::cloud_machine::{self, MachineSnapshot};

#[derive(Default)]
pub(super) struct MachineState {
    loading: bool,
    snapshot: Option<MachineSnapshot>,
    error: Option<String>,
    generation: u64,
    auto_requested: bool,
}

const SETUP_PROMPT: &str = "Help me set up persistent GitHub authentication on this cloud machine. First inspect where my home directory is mounted and whether it survives machine restarts and replacement. Check whether GitHub CLI is installed and authenticated without printing tokens, private keys, credential files, or secret environment values. Prefer a browser/device authorization flow that I approve myself. Before storing credentials, explain whether a secure credential store exists or whether storage would be plaintext, and ask for my approval if plaintext storage is necessary. Do not copy credentials from another machine, request secrets in chat, or overwrite existing credentials. Once approved and authenticated, configure Git to use the GitHub CLI credential helper, verify access without printing secrets, and explain what survives a restart and how to revoke access.";

impl MobileApp {
    pub(super) fn ensure_computer_stats(&mut self, cx: &mut Context<Self>) {
        if self.offline || self.account.is_none() || self.cloud_machine.auto_requested {
            return;
        }
        self.cloud_machine.auto_requested = true;
        self.refresh_cloud_machine(false, cx);
    }

    fn inspect_cloud_machine(&mut self, cx: &mut Context<Self>) {
        self.refresh_cloud_machine(true, cx);
    }

    fn refresh_cloud_machine(&mut self, interactive: bool, cx: &mut Context<Self>) {
        if self.offline || self.cloud_machine.loading {
            return;
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        if interactive {
            haptics::play(Haptic::Light);
        }
        static NEXT_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        self.cloud_machine.generation =
            NEXT_REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let generation = self.cloud_machine.generation;
        self.cloud_machine.loading = true;
        self.cloud_machine.error = None;
        let expected = account.clone();
        let task = cx.background_spawn(async move { cloud_machine::inspect(&account) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.account.as_ref() != Some(&expected)
                    || this.cloud_machine.generation != generation { return; }
                this.cloud_machine.loading = false;
                match result {
                    Ok(snapshot) => this.cloud_machine.snapshot = Some(snapshot),
                    Err(_) => {
                        if interactive { haptics::play(Haptic::Error); }
                        this.cloud_machine.error = Some("Could not inspect your cloud machine. Check your connection and try again.".into());
                    }
                }
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }

    /// A quiet snapshot, not a live utilization monitor. Never show local-device stats.
    pub(super) fn render_computer_stats(&self, t: &Theme) -> Option<AnyElement> {
        self.account.as_ref()?;
        let state = &self.cloud_machine;
        let resources = state.snapshot.as_ref().and_then(|s| s.resources.as_ref());
        let mut card = div()
            .id("cloud-computer-stats")
            .debug_selector(|| "cloud-computer-stats".into())
            .flex_none()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(px(14.))
            .bg(t.surface)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(
                        svg()
                            .path("icons/cloud.svg")
                            .size(px(13.))
                            .text_color(t.text_muted),
                    )
                    .child(
                        label(
                            computer_title(self.account.as_ref().and_then(|a| a.email.as_deref())),
                            t.text_muted,
                            12.,
                        )
                        .truncate(),
                    ),
            );
        if let Some(r) = resources {
            let stats = [
                (
                    "CPU",
                    format!(
                        "{} vCPU{}",
                        r.cpu_count,
                        if r.cpu_count == 1 { "" } else { "s" }
                    ),
                ),
                (
                    "RAM",
                    capacity(r.memory_available_bytes, r.memory_total_bytes),
                ),
                ("Disk", capacity(r.disk_available_bytes, r.disk_total_bytes)),
            ];
            let mut row = div().flex().gap(px(10.));
            for (name, value) in stats {
                row = row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(label(name, t.text_muted, 10.))
                        .child(label(value, t.text, 11.).truncate()),
                );
            }
            card = card.child(row).child(label(
                "RAM available · Disk free · At last refresh",
                t.text_muted,
                10.,
            ));
        } else {
            card = card.child(label(
                if state.loading {
                    "Checking your cloud computer…"
                } else {
                    "Computer stats unavailable · Refresh in Settings"
                },
                t.text_muted,
                11.,
            ));
        }
        Some(card.into_any_element())
    }

    pub(super) fn render_cloud_machine(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.cloud_machine;
        let mut section = div().flex().flex_col().gap(px(10.))
            .child(label("Cloud machine & saved files", t.text_strong, 13.))
            .child(label("Inspect your cloud home and GitHub sign-in without showing secret contents. Refresh may wake your machine.", t.text_muted, 11.))
            .child(outline_button("inspect-cloud-machine", if state.loading { "Checking cloud machine…" } else { "Refresh cloud machine" }, self.account.is_some() && !state.loading, t)
                .on_click(cx.listener(|this, _, _, cx| this.inspect_cloud_machine(cx))));
        if let Some(snapshot) = &state.snapshot {
            section = section.child(label("Last successful inspection", t.text_muted, 11.));
            let rows = [
                ("Home directory", snapshot.home.clone()),
                (
                    "GitHub CLI",
                    if snapshot.github_cli {
                        "Installed"
                    } else {
                        "Not installed"
                    }
                    .into(),
                ),
                (
                    "GitHub access",
                    if snapshot.github_authenticated {
                        "Authenticated at refresh"
                    } else {
                        "Not authenticated / unavailable"
                    }
                    .into(),
                ),
                (
                    "Saved GitHub config",
                    present(snapshot.github_config_present).into(),
                ),
                ("Possible SSH key files", snapshot.ssh_key_count.to_string()),
                (
                    "Git credential file",
                    present(snapshot.git_credentials_present).into(),
                ),
            ];
            for (name, value) in rows {
                section = section.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(label(name, t.text_muted, 11.))
                        .child(
                            div()
                                .text_size(crate::system::fs(12.))
                                .text_color(t.text)
                                .child(value),
                        ),
                );
            }
            section = section.child(label(
                "Home directory entries (names only)",
                t.text_strong,
                12.,
            ));
            if snapshot.entries.is_empty() {
                section = section.child(label("No entries reported.", t.text_muted, 11.));
            }
            for name in &snapshot.entries {
                section = section.child(
                    div()
                        .text_size(crate::system::fs(11.))
                        .text_color(t.text)
                        .child(name.clone()),
                );
            }
            section = section.child(label("A saved file does not prove valid access. SSH key files may not be registered with GitHub. This inventory does not verify backup or survival after machine replacement.", t.text_muted, 11.));
        }
        if let Some(error) = &state.error {
            section = section.child(label(error.clone(), t.error, 11.));
        }
        section
            .child(label("For persistent access, credentials must be saved on the cloud machine, not just your phone or laptop. GitHub CLI can fall back to plaintext storage when no keyring is available. Never paste tokens or private keys into chat.", t.text_muted, 11.))
            .child(outline_button("setup-cloud-github", "Draft GitHub setup request", self.account.is_some(), t)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.offline || this.account.is_none() { return; }
                    // Preserve any draft already in progress. This never sends a message.
                    let existing = this.composer.read(cx).text().to_string();
                    let draft = setup_draft(&existing);
                    this.composer.update(cx, |input, cx| input.set_text(&draft, cx));
                    haptics::play(Haptic::Light);
                    this.screen = Screen::Chat;
                    cx.notify();
                })))
            .child(label("Opens a draft for you to review and send. It does not install tools or change credentials automatically.", t.text_muted, 11.))
            .into_any_element()
    }
}

fn computer_title(email: Option<&str>) -> String {
    let name = email
        .and_then(|email| email.split('@').next())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    name.map(|name| format!("{name}'s Jcode computer"))
        .unwrap_or_else(|| "Your Jcode computer".into())
}

fn capacity(available: u64, total: u64) -> String {
    format!(
        "{:.1} / {}",
        available as f64 / (1024. * 1024. * 1024.),
        gib(total)
    )
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1024. * 1024. * 1024.))
}

fn present(value: bool) -> &'static str {
    if value {
        "Present (contents hidden)"
    } else {
        "Not found in standard location"
    }
}

fn setup_draft(existing: &str) -> String {
    if existing.trim().is_empty() {
        SETUP_PROMPT.to_string()
    } else {
        format!("{existing}\n\n{SETUP_PROMPT}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn computer_name_and_capacity_formatting() {
        assert_eq!(
            computer_title(Some("jeremy@example.test")),
            "jeremy's Jcode computer"
        );
        assert_eq!(computer_title(None), "Your Jcode computer");
        assert_eq!(computer_title(Some("@example.test")), "Your Jcode computer");
        assert_eq!(gib(8 * 1024 * 1024 * 1024), "8.0 GiB");
        assert_eq!(
            capacity(4 * 1024 * 1024 * 1024, 8 * 1024 * 1024 * 1024),
            "4.0 / 8.0 GiB"
        );
    }

    #[gpui::test]
    fn computer_card_renders_without_suggestions_and_offline_fetch_stays_silent(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = super::super::navigation_coverage::mount(cx);
        view.update(cx, |app, cx| {
            let (_, feedback) = haptics::capture(|| app.ensure_computer_stats(cx));
            assert!(feedback.is_empty());
            assert!(!app.cloud_machine.loading);
            assert!(!app.cloud_machine.auto_requested);
            assert!(app.session.session_list.is_empty());
        });
        super::super::navigation_coverage::draw(cx);
        assert!(cx.debug_bounds("cloud-computer-stats").is_some());
        assert!(cx.debug_bounds("start-suggestions").is_none());
        view.update(cx, |app, cx| {
            app.cloud_machine.snapshot = Some(MachineSnapshot {
                resources: Some(jcode_mobile_core::cloud_machine::MachineResources {
                    cpu_count: 8,
                    memory_total_bytes: 16 * 1024 * 1024 * 1024,
                    memory_available_bytes: 8 * 1024 * 1024 * 1024,
                    disk_total_bytes: 100 * 1024 * 1024 * 1024,
                    disk_available_bytes: 50 * 1024 * 1024 * 1024,
                }),
                home: "/home/fixture".into(),
                entries: vec![],
                github_cli: false,
                github_authenticated: false,
                github_config_present: false,
                ssh_key_count: 0,
                git_credentials_present: false,
            });
            cx.notify();
        });
        super::super::navigation_coverage::draw(cx);
        let card = cx
            .debug_bounds("cloud-computer-stats")
            .expect("resource card");
        assert!(card.size.width <= px(390.));
        assert!(card.size.height > px(50.));
        view.update(cx, |app, cx| {
            app.account = None;
            cx.notify();
        });
        super::super::navigation_coverage::draw(cx);
        assert!(cx.debug_bounds("cloud-computer-stats").is_none());
    }

    #[test]
    fn setup_requires_approval_and_forbids_secret_disclosure() {
        assert!(SETUP_PROMPT.contains("ask for my approval"));
        assert!(SETUP_PROMPT.contains("Do not copy credentials"));
        assert!(SETUP_PROMPT.contains("whether it survives"));
    }

    #[test]
    fn setup_preserves_existing_composer_draft() {
        assert_eq!(setup_draft(""), SETUP_PROMPT);
        let existing = "My unsent work 👩🏽‍💻\nKeep this.";
        assert_eq!(
            setup_draft(existing),
            format!("{existing}\n\n{SETUP_PROMPT}")
        );
    }

    #[gpui::test]
    fn cloud_settings_render_offline_and_clear_inventory_on_logout(cx: &mut gpui::TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            MobileApp::new_offline(
                SafeArea {
                    top: 20.,
                    bottom: 0.,
                },
                window,
                cx,
            )
        });
        view.update(cx, |app, cx| {
            app.account = Some(Account {
                api_key: "fixture-not-a-real-key".into(),
                api_base: "http://127.0.0.1:9".into(),
                email: Some("fixture@example.test".into()),
            });
            app.cloud_machine.snapshot = Some(MachineSnapshot {
                home: "/home/cloud".into(),
                entries: vec![".ssh".into(), "project".into()],
                github_cli: true,
                github_authenticated: false,
                github_config_present: true,
                ssh_key_count: 1,
                git_credentials_present: false,
                resources: None,
            });
            app.screen = Screen::Settings;
            let (_, feedback) = haptics::capture(|| app.inspect_cloud_machine(cx));
            assert!(feedback.is_empty(), "offline inspection must stay silent");
            assert!(
                !app.cloud_machine.loading,
                "offline UI must not reach cloud"
            );
            cx.notify();
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        view.update(cx, |app, cx| {
            app.sign_out(cx);
            assert!(app.cloud_machine.snapshot.is_none());
            assert!(!app.cloud_machine.loading);
        });
    }
}
