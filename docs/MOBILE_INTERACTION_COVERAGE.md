# Mobile interaction coverage inventory

## What this records

`MOBILE_INTERACTION_COVERAGE.json` is the reviewed, machine-readable ledger.
`src/app/coverage_inventory.rs` uses `syn` to follow the Rust module graph from
`src/main.rs` and `src/lib.rs` when present. It inventories production functions
that render UI, own `on_*` / `capture_*` call sites, declare handlers with those
prefixes, or construct identifiable control families. Test-only modules,
functions, and their external files are excluded from production counts.
Regression references must name a reachable function with `#[test]` or
`#[gpui::test]`, as discovered from the parsed source rather than text matching.

The reviewed snapshot records **67 owners, 112 static event-call sites, and
175 control/helper-family sites**. Of those owners, 64 have named, explicitly
partial regression evidence and 3 are gap-only. These are inventory counts,
not a behavioral coverage percentage. Each JSON row declares `coverage_scope`
as `partial` or `gap_only`.

This is a change detector and coverage checklist, not a proof of interaction
correctness. A row with named tests does not mean every callback in that
function is exercised. Evidence notes describe tested subsets, with remaining
gaps or device requirements recorded explicitly. No claim is made that every
interleaving, viewport, accessibility action, platform callback, or live service
response has been covered.

### Evidence categories

- **`hit_tested`**: an existing test injects input into rendered GPUI controls.
  Read the note for the actual route exercised. Bounds-only rendering assertions
  are not labeled hit-tested merely because the control has a debug selector.
- **`state_model_mocked`**: a test directly invokes a model/action/completion
  seam, injects fake service state/results, or asserts rendered layout without
  driving that callback through its hit target. This is not native OS or live
  network coverage.
- **`platform_device_required`**: the remaining behavior needs an iPhone,
  simulator with the native bridge, OS permission/UI, credentials, or an
  explicitly authorized service session. No automated pass is implied.
- **`gap`**: an interaction or visual branch has no focused regression evidence
  recorded here. A broad fuzz test or another test in the same file is not
  silently promoted to full coverage.

## Human checklist

The names below are entry points into the existing regression suite. The JSON
uses unambiguous file/module-qualified names and validates that they still
exist. These sections cover the three application screens plus overlays and
external entry points.

| Surface / action family | Existing evidence | Remaining boundary |
| --- | --- | --- |
| Sign-in screen, provider layout | `phone_sign_in_offers_apple_and_google_before_email_code` checks rendered provider/form layout | Actual Apple and Google buttons launch native/browser flows. Provider success, permission denial, and callback delivery still need platform/service checks. |
| Email code form and use-other-email | `email_auth_outcomes_retry_cancel_and_stale_success_are_isolated` injects auth completions and hit-tests the local cancellation control | Delivery of an email, account persistence, real verification, and provider failures are not exercised. |
| Browser login cancellation | `browser_cancel_old_failure_and_old_success_cannot_replace_new_login` injects old/current completion identities | Native browser launch, OAuth return URLs, and real cancellation timing need a platform run. |
| Interactive sign-in illustration | `raw_touch_rotates_the_rendered_donut` drives touch through the rendered illustration | Cross-platform mouse capture, multi-pointer conflicts, and animation smoothness need manual checks. |
| Chat transcript and follow state | `live_tools_release_follow_on_small_upward_scroll`, `attached_cloud_history_and_footer_stay_inside_phone` check scroll/follow and narrow layouts | Live socket streaming and every tool/markdown branch are not implied by fixture rendering. |
| Composer, typed draft, send/stop | `composer_send_disabled_for_disconnected_missing_session_and_blank_input`, `composer_send_records_local_message_then_queues_processing_followup`, `composer_stop_requires_ready_session_and_does_not_fake_cancel_ack` hit-test readiness, local send/queue and stop-request behavior | No socket exists. Local optimistic intent is not server receipt, interruption, cancellation acknowledgment, or native audio capture. |
| Transcript tool/jump/error controls | `transcript_tool_control_expands_and_collapses_visible_output`, `transcript_jump_control_restores_follow_without_discarding_draft`, `transcript_and_empty_error_banners_dismiss_safely_offline` hit-test all three focused control families | Every tool status, streamed content update and long-output nesting combination is not enumerated. |
| Composer metadata routes | `metadata_picker_actions_preserve_each_tabs_unsent_draft` hit-tests model, account and location paths while switching between draft-owning tabs | Effort is a decorative label with no event handler. Every actual model/account route-availability combination is not enumerated. |
| Header, session tabs, drawer entry | `header_new_session_is_inert_without_account_and_parks_existing_draft` hit-tests header new-session for signed-out/signed-in and draft restore. `drawer_new_switch_close_refresh_and_account_hit_test` exercises drawer/tab navigation controls | Native long-hold timing, accessibility activation, and every tab overflow viewport are not exhaustive. |
| Session drawer, search and directory groups | `drawer_directory_groups_and_search_hit_test`, `drawer_directory_new_button_does_not_toggle_group` hit-test result/group paths after search text is supplied synthetically | `drawer_search_pointer_focus_routes_typed_filter_and_result` separately drives actual pointer focus, routed typing/Home/Delete and result activation. Cloud refresh, native IME and every interruption sequence remain separate coverage. |
| Drawer modal scroll containment | `drawer_scroll_does_not_move_transcript_or_release_follow` injects wheel input across list/pinned/scrim areas | Native momentum and multi-touch transitions still require device checks. |
| Directory sheet and direct edge new session | `directory_sheet_hit_test_invalid_same_valid_and_dismiss`, `directory_recent_row_and_start_suggestions_hit_test`, `right_edge_new_session_has_no_directory_sheet_and_preserves_draft` | Actual remote path validation and directory listing use services. Fixtures do not prove the remote directory exists. |
| Model picker and route variants | `model_picker_bare_selection_same_selection_and_dismiss_hit_test`, `model_picker_unavailable_route_is_inert_and_available_route_selects` | Provider provisioning and actual model changes across a live connection are not exercised. |
| Tab action menu | `tab_menu_switch_close_others_and_disabled_fork_hit_test` and `tab_menu_owns_overlaps_and_local_actions` cover switch/close-others/disabled-fork, direct dismiss, synthetic clipboard copy, last/current close variants and overlapping underlying controls | Enabled remote fork, native clipboard ownership, native hold recognition, and every destructive-close interleaving are separate checks. |
| Message hold menu and retry | `raw_message_hold_copy_and_cancel_preserve_transcript_and_draft`, `raw_disabled_retry_is_inert_and_enabled_retry_resends_correct_prompt`, `stale_enabled_retry_handler_rechecks_processing_before_sending` | Fixture local intents do not prove server retry delivery or native clipboard behavior. |
| Read-only text selection | `selection_is_read_only_copyable_and_done_releases_focus`, `read_only_transcript_can_select_and_copy_but_not_edit` | Native selection handles, iOS copy menu, VoiceOver, and keyboard hardware combinations need device checks. |
| Rename sheet | `rename_save_cancel_empty_unicode_and_settled_keyboard_viewports`, `rename_submit_commits_once_and_releases_editor_focus`, `rename_transition_discards_selection_menu_and_pending_gesture` | Real keyboard show/hide timing, predictive input and server rename acknowledgment are not exercised. |
| Git handle and read-only panel | `raw_touch_pull_opens_git_without_reconnecting_or_recording`, `git_pull_threshold_cancel_and_session_invalidation` drive tap/pull/cancel and context changes | Actual SSH/cloud Git inspection is intentionally absent. No fetch, pull command, mutation, or remote service is authorized by these tests. |
| Git refresh and stale results | `mocked_git_results_reject_stale_refresh_account_tab_session_and_directory` uses the real completion seam with fake snapshots/results | Process/network timeout, host authentication, and real repository output are outside these tests. |
| Git overlay scroll containment | `git_overlay_wheel_at_boundaries_never_scrolls_or_releases_transcript` dispatches wheel phases to panel/header | Device momentum, large remote snapshots, and pointer interaction combinations are not exhaustive. |
| Voice scene eligibility and navigation | `scene_voice_eligibility_excludes_every_modal_and_transient_state`, `tab_switch_and_signout_cancel_mocked_pending_voice` directly inspect safe state transitions | Eligibility is tested without actually starting a microphone. Recording ownership still needs native capture tests. |
| Voice startup/release/lock/cancel | `mocked_startup_failure_release_lock_and_retry_states` and existing voice-control tests seed pending capture state and inject failure/release actions | Real permission startup, successful `Recording` acquisition, audio interruption/background callbacks, WAV stop, and the five-minute recording timer require device tests. |
| Voice result/send/draft isolation | `voice_results_preserve_typing_and_never_send`, `quick_voice_send_preserves_unrelated_draft_and_queues_while_processing`, `mocked_callbacks_after_tab_roundtrip_and_signout_are_stale` inject transcript results | Real transcription/upload, microphone audio quality, and server receipt are not covered. |
| Settings screen navigation and account controls | `settings_new_clear_delete_guard_and_signout_controls_are_hit_tested` drives reachable controls with scrolling and raw input | Account deletion confirmation/service execution and persisted credentials need an explicitly authorized integration run. |
| Apple existing-account link panel | `rendered_link_flow_error_retry_busy_code_done`, `rendered_cancel_reopen_rejects_stale_send_and_verify` hit-test the local panel with fake completions; `offline_link_events_cannot_update_a_different_account` checks parent account identity | Actual email/link verification, native Apple authorization and subscription merge/persistence require service/device checks. |
| Synced-session continuation overlay | `continuation_controls_hit_test_offline_busy_uncertain_and_completed`, `local_sidebar_discloses_more_and_searches_hidden_sessions` drive local disclosure/search/continuation controls | Remote transfer and real continuation acknowledgment are not performed. |
| Settings cloud inventory and GitHub setup draft | `cloud_settings_render_offline_and_clear_inventory_on_logout`, `setup_preserves_existing_composer_draft`, `setup_requires_approval_and_forbids_secret_disclosure` | Real cloud inventory/SSH and credential/browser authorization are deliberately excluded. Tests do not verify actual GitHub authentication. |
| Sign-out lifecycle cleanup | `logout_clears_transient_ui_focus_and_cross_account_pending_actions` and voice/navigation tests assert transient cleanup | OS push token deregistration, account-file deletion, active sockets, and real concurrent auth callbacks need isolated service checks. |
| Deferred deep-link / push prompts across tabs | `pending_external_prompts_stay_owned_by_their_session_tabs` checks ownership. `deferred_prompt_dispatch_waits_for_ready_session_and_preserves_live_draft` injects readiness, invokes the local dispatch seam and checks one-shot intent plus draft/selection preservation | Direct mocked dispatch is not a real connection pump or server receipt. |
| Deep-link parser and cold/warm dispatch | `offline_parser_fixtures_do_not_install_global_url_channels`, `cold_signed_out_links_never_queue_prompts_or_create_tabs`, `warm_links_reuse_active_and_parked_sessions_and_replace_pending_prompt` | Direct parse/dispatch only. OS URL registration, pre-install cold-start buffering, and actual external URL delivery are not tested. |
| APNs foreground/token/open/reply | `offline_token_and_foreground_are_local_only_even_with_account`, `mocked_connected_and_processing_inline_replies_preserve_draft`, `offline_disconnected_reply_waits_and_parked_session_is_reused` | Fake push events only. Notification permission, APNs entitlement/token registration, real badge reset, and actionable notification delivery require a device/service run. |
| Live Activity / Dynamic Island | Existing `live_state_tracks_activity` tests derive a pure state value | Native widget donation/start/update/end and background timing need native bridge/device tests. |
| Simulated keyboard and text input | `seeded_unicode_handler_sequences`, `marked_selection_uses_inserted_text_not_unicode_prefix`, `return_key_submits_not_inserts`, `keyboard_config_reaches_platform` exercise GPUI input/model contracts | GPUI test-platform configuration is not UIKit keyboard proof. Native keyboard layout, secure entry, autocorrection UI, IME, dictation, hardware keys, and selection handles need device checks. |
| Keyboard inset/gesture overlays | Rename and navigation viewport fixtures check settled simulated geometry, not a native keyboard | Native safe-area/rotation/interactive-dismiss timing and accessibility keyboard activation remain separate requirements. |
| Share sheet, clipboard, haptics, OS index | Existing fixture tests may record local intent or inspect haptic counters | Native share presentation, actual clipboard ownership, physical feedback, Spotlight/indexing, and privacy permission behavior are platform requirements, not fixture passes. |

## Guard behavior and maintenance

The normal test `production_event_inventory_matches_reviewed_coverage_ledger`
fails on an unlisted production renderer/handler/control owner, changed event
site counts or control families, stale ledger owners, missing evidence notes,
and stale named-test references. It does not install platform channels, mutate
environment variables, launch services, or bless entries automatically.

For discovery only, explicitly run:

```sh
justrust test -p jcode-mobile dump_interaction_coverage_inventory -- --ignored --nocapture
```

The ignored test prints JSON between `INTERACTION_INVENTORY_BEGIN` and
`INTERACTION_INVENTORY_END`. It writes no files. Review changed functions,
identify actual test assertions and input routes, and then edit the ledger.
A newly added row needs either named evidence with a specific tested-subset
note, or an explicit, interaction-specific gap/device requirement. Placeholder
`REVIEW REQUIRED` notes are rejected by the guard.

Run the guard and its synthetic self-tests after editing:

```sh
justrust test -p jcode-mobile coverage_inventory
```

The self-tests intentionally add new event wiring, a new control family, a new
renderer, empty evidence, a removed owner, and a stale regression name to
in-memory AST/ledger fixtures and assert validation fails. They also verify
that comments, string literals, and test-only functions/modules are not counted
as production wiring. These are tests of the guard, not mobile UI evidence.

### Static-analysis limits

- Renderer discovery uses `render` / `render_*` names and known GPUI element
  return types, including `impl IntoElement`. New naming/type/helper conventions
  require extending the scanner rather than assuming automatic detection.
- Counts are static call-site counts per file/function/impl owner, not individual
  runtime controls. Loops and conditional branches can create many instances
  from a single site. Both iOS and desktop source branches are inventoried even
  on Linux, so the ledger is a target-union view.
- Literal `id`/`debug_selector` values, format-string and tuple ID families, and
  the known `pill`/`outline_button`/`round_button`/nonempty-`row` helper call
  families are recorded.
  Arbitrary expressions are explicitly marked dynamic. A renamed variable can
  require review even if runtime behavior is unchanged.
- Replacing one callback with another while preserving method/family counts can
  escape structural drift detection. Semantic changes still need review/tests.
- Macro expansion, generated code, Rust dependency internals, Swift/Objective-C
  event registration, call-graph reachability and arbitrary UI builder helpers
  are not inferred. New helper conventions need corresponding discovery logic
  and guard self-tests. Native services remain listed above even when they do
  not expose Rust `on_*` calls.
- The source module graph is followed, not every detached `.rs` file on disk.
  External test files are parsed only to validate regression references. A file
  absent from the application module graph is not a production renderer.
- A syntactically present named test is not evidence that it passed in the
  current environment. Build/test results must be reported separately.

### Full-suite checkpoint before additional local regressions

The complete headless matrix at the earlier checkpoint passed **209 tests in each of eight configurations**,
**1,672 passing executions**, with this ledger guard enabled and no filters/skips.
The only ignored test was the opt-in read-only discovery dump. Final receipt:
`20261106-014504189-545062`. Binary/test Clippy also passed, receipt
`20261106-014128835-499822` (warnings remain). See `MOBILE_FUZZING.md` for
raw-corpus replay, minimized model-sheet click-through evidence, and fixture timing
boundaries. This acceptance used local GPUI overrides and does not clear the
explicit native/service/visual gaps above.

### Additional local findings after that checkpoint

Continuing the explicit local-gap review found directory-sheet, Git-panel and
tab-menu click-through as well as stale text-selection dragging after a release
outside the field. These are additional real interaction bugs, not evidence
that the earlier finite campaign was exhaustive. Directory/Git/tab-menu ownership
regressions passed all eight configurations (three tests per configuration,
phone widths 320 and 390), receipt `20261106-015441426-663809`. Removing the
directory/Git fixes made the final two corresponding regressions fail again,
receipt `20261106-015455664-666726`. Initial uncorrected modal ownership failed
all three tests, receipt `20261106-015148286-632315`.

Additional focused coverage includes connection-phase status text and bounded
phone/landscape presentation; rendered dictation cancellation across synthetic
start/busy flags; offline email/code submit boundaries; actual pointer-focused
composer/search/directory/onboarding editing; and standalone rendered software
keyboard callbacks. A fresh complete matrix now validates these integrated
changes: **224 tests passed in each of eight configurations, 1,792 passing
executions**, with zero failures and zero filtered tests. Only the read-only
inventory discovery dump is ignored. The first receipt is
`20261106-020356082-743189` and the final receipt is
`20261106-020744417-771073`. The campaign took 263.4 seconds end-to-end.
All eight runs include the reviewed inventory guard. This is bounded headless
GPUI acceptance, not a claim of every possible mobile interaction or native
service behavior. Hosted builds remained unavailable through the documented
billing-route 404, so these runs used justrust normal local fallback with local
GPUI overrides.

Focused acceptance receipts for this expansion:

- Input field/pointer workflows: six tests in each of eight configurations
  (48 executions), receipt `20261106-020254122-734308`. Outside-release selection
  first failed in `20261106-015514233-671957` before the minimized fix.
- Local dictation/submit controls: two tests per configuration (16 executions),
  receipt `20261106-020008031-709373`. Submit assertions cover the offline
  boundary, not actual email/code validation or provider readiness.
- Software keyboard: five tests per configuration (40 executions), receipt
  `20261106-020317134-738668`, including three rendered-pointer tests.
- Expanded four-seed journey model, including ready/not-ready deferred sends:
  receipt `20261106-015419890-659361` (1,024 generated model actions).
- Shared core: 55 tests passed in `20261106-020432636-748202`.
- Binary/test Clippy passed in `20261106-020404988-744276`; warnings remain.

Standalone software-keyboard tests use fixed renderer styling. Repeating them
under the eight environment configurations is not evidence that Dynamic Type,
Reduce Motion or appearance visually changes that keyboard. App-level layout
fixtures and native UIKit keyboard acceptance remain distinct.

### Follow-up timing, motion and keyboard bridge hypotheses

The next local-gap iteration found additional concrete issues:

- The onboarding donut ignored Reduce Motion and continuously requested frames.
  Its painter now freezes autonomous orientation/frame requests under that
  preference while still accepting explicit user rotation. Rendered cancellation,
  outside-release and competing-contact tests also pass. Focused matrix: 12 tests
  per configuration, receipt `20261106-021314214-813822`.
- The voice preview replayed every elapsed 1/120-second step, with unbounded work
  after suspension and an f32 increment stall at long uptime. Bounded periodic
  integer ticks now reconstruct only the current cycle. The standalone preview
  also initializes and respects Reduce Motion without microphone access. Clock
  red receipt `20261106-021144839-800433`; motion red receipt
  `20261106-021557860-836994`; final eight-test focused matrix receipt
  `20261106-021756964-853344`.
- Real rendered keyboard-to-app tests exposed drawer paint ordering: its overlay
  covered the simulated keyboard, so search and transfer keys did not type.
  Drawing the keyboard above app overlays fixes the same user-visible behavior
  that standalone keyboard callback tests could not detect. Original red receipt
  `20261106-021856673-861699` had exactly those two bridge failures.

The actual corner-mask paint vertices additionally passed independent symmetry,
arc-radius, area and viewport assertions over 1,024 seeded translated viewports,
receipt `20261106-021246538-808010`. This is geometry evidence, not GPU/pixel
acceptance. The newly partial preview owners still have explicit visual gaps.
The three gap-only owners are native URL/push/startup channel installation,
not an assertion that all other interactions are fully covered.

The seven bridge workflows passed the full default and forced-keyboard matrices,
112 behavioral executions total: final receipts `20261106-022518411-913654` and
`20261106-022540249-917550`. Explicit keyboard disable was checked separately
(`20261106-022521554-914242`); its early-return guards are skips, not interaction
coverage. Return intent and unrelated Unicode drafts are asserted across all
focused editors, including transient rename, transfer and Apple-link email.

An apparent horizontal Apple action overflow was isolated to the seeded Settings
push still moving. Tests settle only that presentation after its first render,
then retain full physical bounds assertions and real scrolling to expose fields.
No speculative parent-sizing change was made.

Fresh complete-suite validation is required for these integrated changes. The
earlier 224-test result remains a checkpoint, not acceptance of newer sources.
The first fresh attempt (`866027ojsx`) caught only inventory drift while another
session extracted shared composer/suggestion renderers and improved neighboring
panels and synced-row refresh stability. Those seven changed owner shapes were
reviewed, not automatically blessed: the ledger now records two shared helper
owners, precise neighboring-panel geometry/inertness evidence and fake refresh
ordering/status-layout evidence. The existing scanner guard passed 4 tests with
1 discovery test ignored (`3021063m81`). A newly compiled full matrix is pending.

### Initial validation receipt

The coordinator ran the inventory guard and synthetic self-tests successfully:
4 passed, 1 discovery test ignored, receipt `20261106-012640436-388875`.
The eight focused composer/header regressions passed separately in receipt
`20261106-012754853-398199`. These receipts are bounded validation evidence,
not a claim that all native/service gaps or every interaction sequence passed.
