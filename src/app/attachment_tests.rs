use super::*;
use crate::app::navigation_coverage::{mount, tap};
use gpui::TestAppContext;
use jcode_mobile_core::ConnectionPhase;

fn photo() -> SessionImage {
    SessionImage::new("image/jpeg", vec![1, 2, 3])
}

#[gpui::test]
fn photo_only_send_attaches_to_prompt_and_clears_draft(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.session.phase = ConnectionPhase::Connected;
        app.session.session_id = Some("photos".into());
        app.session.is_processing = false;
        app.attach.pending.push(photo());
        cx.notify();
    });
    tap(cx, "send");
    view.read_with(cx, |app, _| {
        let prompt = app.session.transcript.last().unwrap();
        assert_eq!(prompt.text, "Photo attached.");
        assert_eq!(app.session.images_for(prompt.id), &[photo()]);
        assert!(app.attach.pending.is_empty());
    });
}

#[gpui::test]
fn attachment_menu_and_remove_work_through_controls(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    tap(cx, "attach-photo");
    view.update(cx, |app, cx| {
        assert!(app.attach.menu);
        app.attach.pending.push(photo());
        cx.notify();
    });
    tap(cx, "attachment-0");
    view.read_with(cx, |app, _| assert!(app.attach.pending.is_empty()));
}

#[gpui::test]
fn late_picker_result_stays_with_parked_tab_and_signout_drops_it(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.attach.target = Some(app.active_tab);
        app.park_active(cx);
        app.active_tab = 42;
        app.on_media(Event::Image(photo()), cx);
        assert!(app.attach.pending.is_empty());
        assert_eq!(app.parked.last().unwrap().attachments, vec![photo()]);
        app.clear_attachments();
        app.on_media(Event::Image(photo()), cx);
        assert!(app.attach.pending.is_empty());
    });
}

#[test]
fn invalid_thumbnail_is_rejected() {
    assert!(thumbnail(b"not a photo", 384).is_none());
}

#[gpui::test]
fn assistant_image_only_row_has_a_visible_inline_preview(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    let pixels = image::RgbaImage::from_pixel(16, 8, image::Rgba([255, 0, 0, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let image = SessionImage::new("image/png", bytes.into_inner());
    let id = image.id;
    let (media_type, data) = image.wire();
    view.update(cx, |app, cx| {
        app.session.apply_synced_messages(serde_json::from_value(serde_json::json!([
            {"seq":22,"role":"assistant","text":"","images":[
                {"media_type":media_type,"data":data}]}
        ])).unwrap());
        cx.notify();
    });
    crate::app::navigation_coverage::draw(cx);
    let selector = Box::leak(format!("assistant-image-{id}").into_boxed_str());
    let bounds = cx.debug_bounds(selector).expect("assistant inline image region");
    assert!(bounds.size.width >= px(240.) && bounds.size.height >= px(240.));
    cx.run_until_parked();
    view.read_with(cx, |app, _| {
        assert!(app.attach.thumbs.0.borrow().get(&id).is_some_and(Option::is_some),
            "generated PNG must decode into a render image, not stay a placeholder");
    });
}

#[test]
fn thumbnail_decodes_png_and_converts_red_to_bgra() {
    let pixels = image::RgbaImage::from_pixel(16, 8, image::Rgba([255, 0, 0, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let thumb = thumbnail(bytes.get_ref(), 4).unwrap();
    assert_eq!(&thumb.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
}

#[gpui::test]
fn disconnected_photo_draft_is_preserved_and_busy_send_queues_it(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx);
    view.update(cx, |app, cx| {
        app.attach.pending.push(photo());
        app.session.phase = ConnectionPhase::Disconnected;
        app.send_draft(cx);
        assert_eq!(app.attach.pending, vec![photo()]);
        app.session.phase = ConnectionPhase::Connected;
        app.session.session_id = Some("photos".into());
        app.session.is_processing = true;
        app.send_draft(cx);
        let prompt = app.session.transcript.last().unwrap();
        assert!(prompt.is_queued);
        assert_eq!(app.session.images_for(prompt.id), &[photo()]);
        assert!(app.attach.pending.is_empty());
    });
}
