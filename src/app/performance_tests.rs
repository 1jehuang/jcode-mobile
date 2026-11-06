//! Real GPUI draw-path regression tests and opt-in same-process benchmarks.
use super::navigation_coverage::{draw, mount};
use super::*;
use gpui::{TestAppContext, VisualTestContext, point};

fn populate(view: &Entity<MobileApp>, cx: &mut VisualTestContext, count: usize) {
    view.update(cx, |app, cx| {
        app.session.phase = ConnectionPhase::Connected;
        app.follow = false;
        app.session.transcript = (0..count).map(|index| TranscriptEntry {
            id: index as u64,
            role: if index % 3 == 0 { Role::User } else { Role::Assistant },
            text: format!("Message {index}\n\nA **formatted** paragraph with `inline code` and Unicode 界🦀.\n\n- first item\n- second item\n\n```rust\nfn example() {{}}\n```"),
            reasoning: String::new(), tool_calls: vec![], is_streaming: false, is_queued: false,
        }).collect();
        cx.notify();
    });
    draw(cx);
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
}

fn redraw(view: &Entity<MobileApp>, cx: &mut VisualTestContext) {
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
}

#[gpui::test]
#[ignore = "cold-layout versus warm-frame performance investigation"]
fn benchmark_cold_transcript_layout(cx: &mut TestAppContext) {
    use std::time::Instant;
    let (view, cx) = mount(cx);
    for count in [90, 500] {
        view.update(cx, |app, cx| {
            app.session.phase = ConnectionPhase::Connected;
            app.follow = false;
            app.session.transcript = (0..count).map(|index| TranscriptEntry {
                // Fresh keys prevent earlier measurements from supplying heights.
                id: (count * 1000 + index) as u64,
                role: Role::Assistant,
                text: format!("Cold workload {count}/{index}\n\nA **formatted** paragraph with `inline code` and Unicode 界🦀.\n\n- first item\n- second item\n\n```rust\nfn example() {{}}\n```"),
                reasoning: String::new(), tool_calls: vec![], is_streaming: false, is_queued: false,
            }).collect();
            cx.notify();
        });
        let start = Instant::now();
        draw(cx);
        let cold = start.elapsed();
        let cold_rows = view.read_with(cx, |app, _| app.rendered_transcript_rows);
        let start = Instant::now();
        for _ in 0..20 { redraw(&view, cx); }
        let warm = start.elapsed();
        let warm_rows = view.read_with(cx, |app, _| app.rendered_transcript_rows);
        eprintln!("cold layout {count} messages: {:.3}ms/{cold_rows} subtrees, warm mean {:.3}ms/{warm_rows} subtrees", cold.as_secs_f64()*1000.0, warm.as_secs_f64()*50.0);
        assert_eq!(cold_rows, count);
        assert!(warm_rows < count);
    }
}

#[gpui::test]
fn windowed_transcript_preserves_geometry_scrolling_edits_and_resize(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    populate(&view, cx, 90);
    for offset in [0., 1200., 8000.] {
        view.update(cx, |app, cx| {
            app.transcript_scroll
                .set_offset(gpui::point(px(0.), px(-offset)));
            cx.notify();
        });
        redraw(&view, cx);
        let (max, scroll, rendered) = view.read_with(cx, |app, _| {
            (
                app.transcript_scroll.max_offset(),
                app.transcript_scroll.offset(),
                app.rendered_transcript_rows,
            )
        });
        assert!(
            rendered < 30,
            "only viewport and overscan should build content, got {rendered}"
        );
        let selectors = [
            "entry-0", "entry-1", "entry-2", "entry-3", "entry-4", "entry-5", "entry-6", "entry-7", "entry-8", "entry-9",
            "entry-10", "entry-11", "entry-12", "entry-13", "entry-14", "entry-15", "entry-16", "entry-17", "entry-18", "entry-19",
            "entry-20", "entry-21", "entry-22", "entry-23", "entry-24", "entry-25", "entry-26", "entry-27", "entry-28", "entry-29",
            "entry-30", "entry-31", "entry-32", "entry-33", "entry-34", "entry-35", "entry-36", "entry-37", "entry-38", "entry-39",
            "entry-40", "entry-41", "entry-42", "entry-43", "entry-44", "entry-45", "entry-46", "entry-47", "entry-48", "entry-49",
            "entry-50", "entry-51", "entry-52", "entry-53", "entry-54", "entry-55", "entry-56", "entry-57", "entry-58", "entry-59",
            "entry-60", "entry-61", "entry-62", "entry-63", "entry-64", "entry-65", "entry-66", "entry-67", "entry-68", "entry-69",
            "entry-70", "entry-71", "entry-72", "entry-73", "entry-74", "entry-75", "entry-76", "entry-77", "entry-78", "entry-79",
            "entry-80", "entry-81", "entry-82", "entry-83", "entry-84", "entry-85", "entry-86", "entry-87", "entry-88", "entry-89",
        ];
        let before: Vec<_> = selectors
            .iter()
            .filter_map(|selector| cx.debug_bounds(selector).map(|bounds| (*selector, bounds)))
            .collect();
        assert_eq!(before.len(), rendered, "all materialized rows must be checked");
        view.update(cx, |app, cx| {
            app.disable_transcript_windowing = true;
            cx.notify();
        });
        draw(cx);
        view.read_with(cx, |app, _| {
            assert_eq!(app.transcript_scroll.max_offset(), max);
            assert_eq!(app.transcript_scroll.offset(), scroll);
            assert_eq!(app.rendered_transcript_rows, 90);
        });
        for (selector, before) in before {
            assert_eq!(
                cx.debug_bounds(selector).unwrap(),
                before,
                "{selector} geometry changed"
            );
        }
        view.update(cx, |app, cx| {
            app.disable_transcript_windowing = false;
            cx.notify();
        });
        draw(cx);
    }
    view.update(cx, |app, cx| {
        app.session.transcript[0].text = "new content\n\n".repeat(50);
        app.session.transcript[89]
            .text
            .push_str("\n\nstreamed tail");
        cx.notify();
    });
    draw(cx);
    redraw(&view, cx);
    view.update(cx, |app, cx| {
        app.device = Some((320., 844.));
        cx.notify();
    });
    cx.simulate_resize(gpui::size(px(320.), px(844.)));
    draw(cx);
    redraw(&view, cx);
    let max = view.read_with(cx, |app, _| app.transcript_scroll.max_offset());
    view.update(cx, |app, cx| {
        app.disable_transcript_windowing = true;
        cx.notify();
    });
    draw(cx);
    view.read_with(cx, |app, _| {
        assert_eq!(app.transcript_scroll.max_offset(), max)
    });
    // A direct large jump must materialize the destination on the next draw.
    view.update(cx, |app, cx| {
        app.disable_transcript_windowing = false;
        app.transcript_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    });
    draw(cx);
    assert!(cx.debug_bounds("entry-0").unwrap().size.height > px(100.));
}

/// Alternate old/new spacer layout within one process, using moving offsets
/// rather than idle redraws. Timings include GPUI layout and paint on a test
/// renderer, not real-device presentation or GPU completion.
#[gpui::test]
#[ignore = "opt-in scrolling draw benchmark, no timing assertion"]
fn benchmark_scrolling_spacer_coalescing(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    populate(&view, cx, 500);
    for coalesced in [false, true, true, false] {
        view.update(cx, |app, cx| {
            app.disable_transcript_coalescing = !coalesced;
            cx.notify();
        });
        draw(cx);
        let mut samples = Vec::new();
        for frame in 0..60 {
            let start = std::time::Instant::now();
            view.update(cx, |app, cx| {
                app.transcript_scroll.set_offset(point(px(0.), px(-1000. - frame as f32 * 80.)));
                cx.notify();
            });
            draw(cx);
            let elapsed = start.elapsed();
            crate::perf::record("benchmark.scroll_draw", elapsed);
            samples.push(elapsed.as_secs_f64() * 1000.);
        }
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        samples.sort_by(f64::total_cmp);
        eprintln!("scroll coalesced={coalesced}, 500 messages, 60 frames: mean={mean:.3}ms p95={:.3}ms max={:.3}ms", samples[56], samples[59]);
    }
}

#[gpui::test]
fn idle_screens_do_not_schedule_spinner_wakeups(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    draw(cx);
    view.update(cx, |app, cx| {
        assert!(app.spinner_tick.is_none());
        app.screen = Screen::SignIn;
        app.browser_login = Some(Default::default());
        assert!(app.needs_spinner_tick());
        app.sync_spinner_tick(cx);
        assert_eq!(app.spinner_tick.is_some(), !crate::system::reduce_motion());
        app.screen = Screen::Settings;
        app.sync_spinner_tick(cx);
        assert!(app.spinner_tick.is_none());
        app.screen = Screen::Chat;
        app.session.phase = ConnectionPhase::Disconnected;
        app.session.is_processing = true;
        app.sync_spinner_tick(cx);
        assert!(
            app.spinner_tick.is_none(),
            "orb activity is not a text spinner"
        );
    });
}

/// Same binary, same chat, warmed caches, exact GPUI draw path.
/// justrust test benchmark_long_transcript_frames -- --ignored --nocapture --test-threads=1
#[gpui::test]
#[ignore = "opt-in same-process draw benchmark, no timing assertion"]
fn benchmark_long_transcript_frames(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    populate(&view, cx, 250);
    for disabled in [true, false, true, false] {
        view.update(cx, |app, cx| {
            app.disable_transcript_windowing = disabled;
            cx.notify();
        });
        draw(cx);
        let start = std::time::Instant::now();
        for _ in 0..20 {
            redraw(&view, cx);
        }
        let rendered = view.read_with(cx, |app, _| app.rendered_transcript_rows);
        eprintln!(
            "transcript windowing={}, 20 redraws: {:?}, built {rendered}/250 rows per frame",
            !disabled,
            start.elapsed()
        );
    }
    view.update(cx, |app, cx| {
        app.park_active(cx);
        let next = app.next_tab;
        app.next_tab += 1;
        app.active_tab = next;
        app.tab_order.push(next);
        app.swipe.feed(100., 0., std::time::Instant::now());
        cx.notify();
    });
    draw(cx);
    for disabled in [true, false] {
        view.update(cx, |app, cx| {
            app.disable_transcript_windowing = disabled;
            app.swipe.last_event = Some(std::time::Instant::now());
            cx.notify();
        });
        draw(cx);
        let start = std::time::Instant::now();
        for _ in 0..20 {
            view.update(cx, |app, cx| {
                app.swipe.last_event = Some(std::time::Instant::now());
                cx.notify();
            });
            draw(cx);
            assert!(
                cx.debug_bounds("panel-preview-0").is_some(),
                "benchmark must keep preview mounted"
            );
        }
        eprintln!(
            "swipe preview windowing={}, 20 redraws: {:?}",
            !disabled,
            start.elapsed()
        );
    }
}
