//! Cold exact-height regression fixtures. These test geometry, not native shaping speed.
use super::super::navigation_coverage::{draw, mount};
use super::super::*;
use gpui::{TestAppContext, VisualTestContext, point};

fn long_entry(index: usize) -> TranscriptEntry {
    let prefix = format!("Cold row {index}: **formatted** `inline code` 界🦀\n\n");
    let paragraph = "A unique transcript keeps all its history accessible and wraps naturally. ";
    let mut text = prefix;
    while text.len() < 4096 {
        text.push_str(paragraph);
    }
    TranscriptEntry {
        id: index as u64,
        role: if index % 5 == 0 {
            Role::User
        } else {
            Role::Assistant
        },
        text,
        reasoning: if index % 11 == 0 {
            "Reasoning that also wraps across the phone viewport. ".repeat(8)
        } else {
            String::new()
        },
        tool_calls: vec![],
        is_streaming: false,
        is_queued: false,
    }
}

fn compare_with_full_layout(view: &Entity<MobileApp>, cx: &mut VisualTestContext, count: usize) {
    let expected = view.read_with(cx, |app, _| {
        (
            app.transcript_scroll.max_offset(),
            app.transcript_scroll.offset(),
        )
    });
    let bounds: Vec<_> = (0..count)
        .filter_map(|index| {
            let selector: &'static str = Box::leak(format!("entry-{index}").into_boxed_str());
            cx.debug_bounds(selector).map(|bounds| (selector, bounds))
        })
        .collect();
    assert!(
        !bounds.is_empty(),
        "the visible destination must be materialized"
    );
    view.update(cx, |app, cx| {
        app.disable_transcript_windowing = true;
        cx.notify();
    });
    draw(cx);
    view.read_with(cx, |app, _| {
        assert_eq!(
            app.rendered_transcript_rows, count,
            "full history must remain available"
        );
        assert_eq!(app.transcript_scroll.max_offset(), expected.0);
        assert_eq!(app.transcript_scroll.offset(), expected.1);
    });
    for (selector, expected) in bounds {
        assert_eq!(
            cx.debug_bounds(selector),
            Some(expected),
            "{selector} shifted"
        );
    }
    view.update(cx, |app, cx| {
        app.disable_transcript_windowing = false;
        cx.notify();
    });
    draw(cx);
}

#[gpui::test]
fn cold_long_rows_preserve_exact_geometry_through_jumps_edits_and_resize(cx: &mut TestAppContext) {
    const COUNT: usize = 40;
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.phase = ConnectionPhase::Connected;
        app.follow = false;
        app.session.transcript = (0..COUNT).map(long_entry).collect();
        app.transcript_heights = Default::default();
        cx.notify();
    });
    draw(cx);
    view.read_with(cx, |app, _| {
        assert_eq!(
            app.rendered_transcript_rows, COUNT,
            "cold geometry cannot use guessed spacers"
        );
        assert!(
            app.session
                .transcript
                .iter()
                .all(|entry| entry.text.len() >= 4096)
        );
    });
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, COUNT);
    for fraction in [0.5, 1., 0.] {
        view.update(cx, |app, cx| {
            let max = app.transcript_scroll.max_offset().y;
            app.transcript_scroll
                .set_offset(point(px(0.), -max * fraction));
            cx.notify();
        });
        draw(cx);
        compare_with_full_layout(&view, cx, COUNT);
    }
    view.update(cx, |app, cx| {
        // Same byte length, different line structure: byte counts cannot stand in for geometry.
        app.session.transcript[1].text = "\n".repeat(app.session.transcript[1].text.len());
        app.session.transcript[COUNT - 1]
            .text
            .push_str("\n\nA newly appended tail.");
        cx.notify();
    });
    draw(cx);
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, COUNT);
    view.update(cx, |app, cx| {
        app.device = Some((320., 844.));
        cx.notify();
    });
    cx.simulate_resize(gpui::size(px(320.), px(844.)));
    draw(cx);
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, COUNT);
}

#[gpui::test]
#[ignore = "opt-in cold 500x4096-byte fixture, headless timings are not native presentation"]
fn benchmark_cold_500_long_rows(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.phase = ConnectionPhase::Connected;
        app.follow = false;
        app.session.transcript = (0..500).map(long_entry).collect();
        app.transcript_heights = Default::default();
        cx.notify();
    });
    let started = std::time::Instant::now();
    draw(cx);
    eprintln!(
        "cold exact-height 500x4096-byte headless draw: {:.3}ms",
        started.elapsed().as_secs_f64() * 1000.
    );
    view.read_with(cx, |app, _| assert_eq!(app.rendered_transcript_rows, 500));
    view.update(cx, |_, cx| cx.notify());
    draw(cx);
    compare_with_full_layout(&view, cx, 500);
}
