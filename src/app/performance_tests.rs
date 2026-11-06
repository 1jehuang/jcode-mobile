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
            "entry-0", "entry-1", "entry-3", "entry-10", "entry-45", "entry-89",
        ];
        let before: Vec<_> = selectors
            .iter()
            .map(|selector| cx.debug_bounds(selector).unwrap())
            .collect();
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
        for (selector, before) in selectors.into_iter().zip(before) {
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
