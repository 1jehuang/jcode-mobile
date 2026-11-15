//! Read-only, explicitly opted-in Android debug acceptance evidence.
//! Never compiled into release app code. All snapshots read the shared GPUI
//! state and submissions are counted at the real local prompt mutation.
use std::{path::Path, time::Duration};
use gpui::Context;
use super::{Account, MobileApp};

const FIXTURE_KEY: &str = "android-offline-share-fixture-not-a-key";
const FIXTURE_BASE: &str = "https://offline-share-fixture.invalid";

#[derive(Default)]
pub(super) struct Probe {
    pub(super) submitted_count: u64,
    sequence: u64,
}

impl MobileApp {
    pub(super) fn prepare_debug_share_fixture(&mut self) {
        assert!(self.offline, "debug share fixture must never load or connect an account");
        assert!(self.connection.is_none() && self.parked.iter().all(|tab| tab.connection.is_none()));
        self.account = Some(Account {
            api_key: FIXTURE_KEY.into(), api_base: FIXTURE_BASE.into(),
            email: None, name: None, given_name: None,
        });
    }

    pub(super) fn debug_share_snapshot(&mut self, cx: &gpui::App) -> serde_json::Value {
        self.debug_share_probe.sequence += 1;
        let synthetic = self.account.as_ref().is_some_and(|a| a.api_key == FIXTURE_KEY && a.api_base == FIXTURE_BASE);
        serde_json::json!({
            "probe": "incoming-share-state-v1",
            "offline": self.offline,
            "syntheticAccount": synthetic,
            "connectionPresent": self.connection.is_some() || self.parked.iter().any(|tab| tab.connection.is_some()),
            "autoSendPending": self.auto_send.is_some() || self.parked.iter().any(|tab| tab.auto_send.is_some()),
            "activeTab": self.active_tab,
            "tabCount": self.tab_order.len(),
            // Never emit private account/user content if the fixture invariant
            // is violated. The harness rejects syntheticAccount=false.
            "composer": if self.offline && synthetic { self.composer.read(cx).text().to_string() } else { String::new() },
            "submittedCount": self.debug_share_probe.submitted_count,
            "frame": self.debug_share_probe.sequence,
        })
    }

    pub(super) fn write_debug_share_snapshot(&mut self, path: &Path, cx: &gpui::App) -> std::io::Result<()> {
        let snapshot = self.debug_share_snapshot(cx);
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, snapshot.to_string())?;
        std::fs::rename(temporary, path)
    }

    pub(super) fn start_debug_share_probe(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        // The state is sampled on the GPUI thread, never from JNI or a worker.
        // Sequence freshness witnesses actual GPUI pumping, not fixed fixtures.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                if this.update(cx, |this, cx| {
                    if this.write_debug_share_snapshot(&path, cx).is_err() {
                        log::warn!("debug share state write failed");
                    }
                }).is_err() { break; }
            }
        }).detach();
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{init, lifecycle_coverage::fixture_app};
    use crate::ios_ext;
    use gpui::TestAppContext;
    use jcode_mobile_core::ConnectionPhase;

    fn has_cfg(attributes: &[syn::Attribute], expected: &str) -> bool {
        attributes.iter().any(|attribute| match &attribute.meta {
            syn::Meta::List(list) if list.path.is_ident("cfg") =>
                list.tokens.to_string().split_whitespace().collect::<String>() == expected,
            _ => false,
        })
    }

    #[test]
    fn release_cfg_excludes_probe_export_storage_and_app_state() {
        let library = syn::parse_file(include_str!("../lib.rs")).unwrap();
        let debug_only = "all(target_os=\"android\",debug_assertions)";
        for name in ["ANDROID_DEBUG_SHARE_PROBE", "android_debug_share_probe_path",
            "Java_org_jcode_mobile_JcodeActivity_nativeConfigureDebugShareProbe"] {
            let attributes = library.items.iter().find_map(|item| match item {
                syn::Item::Static(item) if item.ident == name => Some(&item.attrs),
                syn::Item::Fn(item) if item.sig.ident == name => Some(&item.attrs),
                _ => None,
            }).unwrap();
            assert!(has_cfg(attributes, debug_only), "{name} must never exist in release");
        }
        let app = syn::parse_file(include_str!("../app.rs")).unwrap();
        let probe_cfg = "any(test,all(target_os=\"android\",debug_assertions))";
        assert!(app.items.iter().any(|item| matches!(item,
            syn::Item::Mod(module) if module.ident == "android_debug_probe" && has_cfg(&module.attrs, probe_cfg))));
        let fields = app.items.iter().find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "MobileApp" => Some(&item.fields),
            _ => None,
        }).unwrap();
        let probe = fields.iter().find(|field| field.ident.as_ref().is_some_and(|name| name == "debug_share_probe")).unwrap();
        assert!(has_cfg(&probe.attrs, probe_cfg));
    }

    #[gpui::test]
    fn snapshots_read_real_drafts_and_real_submission_mutations(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(fixture_app);
        view.update(cx, |app, cx| {
            app.prepare_debug_share_fixture();
            let before = app.debug_share_snapshot(cx);
            assert_eq!(before["submittedCount"], 0);
            assert_eq!(before["syntheticAccount"], true);
            let payload = "share ✓ &draft=0";
            app.on_deep_link(ios_ext::parse("jcode://draft?text=share+%E2%9C%93+%26draft%3D0").unwrap(), cx);
            let shared = app.debug_share_snapshot(cx);
            assert_eq!(shared["composer"], payload);
            assert_eq!(shared["submittedCount"], 0);
            assert_eq!(shared["autoSendPending"], false);
            assert_eq!(shared["connectionPresent"], false);
            assert_eq!(shared["tabCount"], 2);
            assert_ne!(shared["activeTab"], before["activeTab"]);
            app.session.phase = ConnectionPhase::Connected;
            app.session.session_id = Some("offline-share-test".into());
            app.flush_pending_send(cx);
            assert_eq!(app.debug_share_snapshot(cx)["submittedCount"], 0);
            // Exercise the real shared submit path, offline so no network/model
            // request can occur. This must increment real observed submissions.
            app.send_draft(cx);
            let sent = app.debug_share_snapshot(cx);
            assert_eq!(sent["submittedCount"], 1);
            assert_eq!(sent["composer"], "");
            assert_eq!(app.session.transcript.last().unwrap().text, payload);
            app.new_tab(cx);
            assert_eq!(app.debug_share_snapshot(cx)["submittedCount"], 1, "tab switch cannot reset real submission history");
            app.auto_send = Some("pending offline".into());
            app.new_tab(cx);
            assert_eq!(app.debug_share_snapshot(cx)["autoSendPending"], true, "parked automatic sends must be observed");
            assert!(app.debug_share_snapshot(cx)["frame"].as_u64() > sent["frame"].as_u64());
        });
    }

    #[gpui::test]
    fn atomic_probe_never_records_nonfixture_private_drafts(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(fixture_app);
        view.update(cx, |app, cx| {
            app.composer.update(cx, |input, cx| input.set_text("private user content", cx));
            assert_eq!(app.debug_share_snapshot(cx)["composer"], "");
            app.prepare_debug_share_fixture();
            let directory = std::env::temp_dir().join(format!("jcode-share-probe-{}-{:?}", std::process::id(), cx.entity_id()));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("incoming-share-state.json");
            app.write_debug_share_snapshot(&path, cx).unwrap();
            let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved["composer"], "private user content");
            assert_eq!(saved["offline"], true);
            assert!(!path.with_extension("json.tmp").exists());
            std::fs::remove_file(path).unwrap();
            std::fs::remove_dir(directory).unwrap();
        });
    }

    #[gpui::test]
    fn sampler_pumps_real_app_state_and_replaces_snapshots_atomically(cx: &mut TestAppContext) {
        cx.update(init);
        let (view, cx) = cx.add_window_view(fixture_app);
        let directory = std::env::temp_dir().join(format!("jcode-share-sampler-{}-{:?}", std::process::id(), view.entity_id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("incoming-share-state.json");
        view.update(cx, |app, cx| {
            app.prepare_debug_share_fixture();
            app.start_debug_share_probe(path.clone(), cx);
        });
        cx.run_until_parked();
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        let before: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        view.update(cx, |app, cx| app.composer.update(cx, |input, cx| input.set_text("real sampled draft", cx)));
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["composer"], "real sampled draft");
        assert!(after["frame"].as_u64() > before["frame"].as_u64());
        drop(view);
        cx.run_until_parked();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
