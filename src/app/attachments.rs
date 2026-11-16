//! Photos attached to the draft, and the thumbnails shown for them.
//!
//! Attachments belong to a tab, like its draft text: they are parked with it
//! and never follow the user into another conversation. A picker that returns
//! after a tab switch still delivers into the tab that opened it.
//!
//! Thumbnails are decoded off the main thread into small bitmaps. Rows keep a
//! fixed size while one is loading, so decoding never moves the transcript.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{AnyElement, Context, RenderImage, prelude::*, div, px};
use jcode_mobile_core::{LocalIntent, Request, SessionImage};

use super::a11y::A11yExt as _;
use super::{MobileApp, outline_button};
use crate::haptics::{self, Haptic};
use crate::media::{self, Event, Source};
use crate::theme::Theme;

/// Each photo is roughly a megabyte on the wire after base64.
pub(super) const MAX_ATTACHMENTS: usize = 4;
const THUMB_PX: u32 = 384;
const MAX_THUMBS: usize = 64;
const PROMPT_PHOTO: f32 = 96.;
const DRAFT_PHOTO: f32 = 56.;

#[derive(Default)]
pub(super) struct State {
    /// Photos on the active tab's draft.
    pub(super) pending: Vec<SessionImage>,
    /// The camera / library choice is showing.
    pub(super) menu: bool,
    /// Tab that opened the system picker.
    target: Option<u64>,
    thumbs: Thumbs,
}

/// Decoded thumbnails retain loading and failure separately, so broken media
/// never remains an unexplained empty rectangle.
#[derive(Clone)]
enum Thumb {
    Loading,
    Ready(Arc<RenderImage>),
    Unavailable,
}

#[derive(Clone, Default)]
struct Thumbs(Rc<RefCell<HashMap<u64, Thumb>>>);

/// A small BGRA bitmap for `bytes`, or `None` when they are not an image.
pub(super) fn thumbnail(bytes: &[u8], max: u32) -> Option<RenderImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    if decoded.width() == 0 || decoded.height() == 0 {
        return None;
    }
    let decoded = if decoded.width() > max || decoded.height() > max {
        decoded.thumbnail(max, max)
    } else {
        decoded
    };
    let mut pixels = decoded.into_rgba8();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(RenderImage::new(vec![image::Frame::new(pixels)]))
}

/// The text sent for a photo with no caption. The server drops an empty
/// prompt from history, which would also drop its photos.
pub(super) fn caption(count: usize) -> String {
    if count == 1 { "Photo attached.".into() } else { format!("{count} photos attached.") }
}

impl MobileApp {
    pub(super) fn install_media(&mut self, cx: &mut Context<Self>) {
        let mut rx = media::install();
        cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                if this.update(cx, |this, cx| this.on_media(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn on_media(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Image(image) => {
                let Some(tab) = self.attach.target else { return };
                let pending = if tab == self.active_tab {
                    Some(&mut self.attach.pending)
                } else {
                    self.parked.iter_mut().find(|p| p.id == tab).map(|p| &mut p.attachments)
                };
                if let Some(pending) = pending
                    && pending.len() < MAX_ATTACHMENTS
                    && !pending.iter().any(|p| p.id == image.id)
                {
                    pending.push(image);
                }
            }
            Event::Done | Event::Cancelled => self.attach.target = None,
            Event::Unavailable => {
                self.attach.target = None;
                self.toast("Photo unavailable. For camera access, check Settings.");
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_attach_menu(&mut self, cx: &mut Context<Self>) {
        haptics::play(Haptic::Light);
        self.attach.menu = !self.attach.menu;
        cx.notify();
    }

    pub(super) fn pick_photo(&mut self, source: Source, cx: &mut Context<Self>) {
        self.attach.menu = false;
        let room = MAX_ATTACHMENTS.saturating_sub(self.attach.pending.len());
        if room == 0 {
            self.toast("Up to 4 photos per message");
        } else if self.attach.target.is_none() {
            self.attach.target = Some(self.active_tab);
            if !media::pick(source, room) {
                self.attach.target = None;
                self.toast("Photos are not available here");
            }
        }
        cx.notify();
    }

    pub(super) fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.attach.pending.len() {
            haptics::play(Haptic::Light);
            self.attach.pending.remove(index);
            cx.notify();
        }
    }

    /// Forget every photo, including decoded thumbnails (sign-out).
    pub(super) fn clear_attachments(&mut self) {
        self.attach = State::default();
    }

    /// Adds the prompt locally and sends it, queued when a turn is running.
    pub(super) fn send_prompt(&mut self, text: String, images: Vec<SessionImage>) {
        #[cfg(any(test, all(target_os = "android", debug_assertions)))]
        { self.debug_share_probe.submitted_count += 1; }
        let queued = self.session.is_processing;
        self.session.apply_intent(if queued {
            LocalIntent::UserQueuedInterrupt(text.clone())
        } else {
            LocalIntent::UserSentMessage(text.clone())
        });
        self.session.attach_images_to_last_prompt(images.clone());
        self.sync_turn_keepalive();
        match (queued, images.is_empty()) {
            (false, true) => self.send(|id| Request::Message { id, content: text }),
            (false, false) => self.send(|id| Request::MessageWithImages { id, content: text, images }),
            (true, true) => self.send(|id| Request::SoftInterrupt { id, content: text, urgent: false }),
            (true, false) => self.send(|id| Request::SoftInterruptWithImages { id, content: text, images, urgent: false }),
        }
    }

    fn thumb(&self, image: &SessionImage, cx: &mut Context<Self>) -> Thumb {
        if let Some(slot) = self.attach.thumbs.0.borrow().get(&image.id) {
            return slot.clone();
        }
        {
            let mut map = self.attach.thumbs.0.borrow_mut();
            if map.len() >= MAX_THUMBS {
                for (_, old) in map.drain() {
                    if let Thumb::Ready(old) = old {
                        cx.drop_image(old, None);
                    }
                }
            }
            map.insert(image.id, Thumb::Loading);
        }
        let (id, bytes, thumbs) = (image.id, image.bytes.clone(), self.attach.thumbs.clone());
        let decode = cx.background_spawn(async move { thumbnail(&bytes, THUMB_PX) });
        cx.spawn(async move |this, cx| {
            let decoded = decode.await.map(|image| Thumb::Ready(Arc::new(image))).unwrap_or(Thumb::Unavailable);
            thumbs.0.borrow_mut().insert(id, decoded);
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
        Thumb::Loading
    }

    fn photo(&self, image: &SessionImage, side: f32, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match self.thumb(image, cx) {
            Thumb::Ready(data) => gpui::img(gpui::ImageSource::Render(data))
                .size(px(side))
                .flex_none()
                .rounded(px(8.))
                .object_fit(gpui::ObjectFit::Contain)
                .into_any_element(),
            state => div()
                .debug_selector({ let id = image.id; move || format!("image-placeholder-{id}") })
                .size(px(side)).flex_none().rounded(px(8.)).bg(t.surface_elevated)
                .flex().items_center().justify_center().p(px(6.))
                .text_size(crate::system::fs(11.)).text_color(t.text_muted)
                .child(if matches!(state, Thumb::Unavailable) { "Image unavailable" } else { "Loading image…" })
                .into_any_element(),
        }
    }

    /// The photos of a sent prompt, shown above its text.
    pub(super) fn render_prompt_photos(&self, images: &[SessionImage], t: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        if images.is_empty() {
            return None;
        }
        Some(div().flex().flex_wrap().gap(px(4.)).pt(px(4.)).children(
            images.iter().map(|image| self.photo(image, PROMPT_PHOTO, t, cx)).collect::<Vec<_>>(),
        ))
    }

    /// Generated and tool-result images, with a readable inline preview.
    pub(super) fn render_assistant_photos(&self, images: &[SessionImage], t: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        if images.is_empty() {
            return None;
        }
        let first = images[0].id;
        Some(div().debug_selector(move || format!("assistant-image-{first}"))
            .flex().flex_wrap().gap(px(6.)).pt(px(4.)).children(
            images.iter().map(|image| self.photo(image, 240., t, cx)).collect::<Vec<_>>(),
        ))
    }

    /// Rows above the composer field: the source choice and the draft's photos.
    pub(super) fn render_attachment_tray(&self, pending: &[SessionImage], preview: bool, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let menu = self.attach.menu && !preview;
        if !menu && pending.is_empty() {
            return None;
        }
        let mut tray = div().flex().flex_col().gap(px(8.)).px(px(4.));
        if !pending.is_empty() {
            let chips = pending.iter().enumerate().map(|(i, image)| {
                div()
                    .id(("attachment", i))
                    .debug_selector(move || format!("{}attachment-{i}", if preview { "preview-" } else { "" }))
                    .relative()
                    .flex_none()
                    .child(self.photo(image, DRAFT_PHOTO, t, cx))
                    .child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .right(px(2.))
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(t.surface)
                            .text_color(t.text_strong)
                            .text_size(crate::system::fs(11.))
                            .child("×"),
                    )
                    .when(!preview, |d| {
                        d.cursor_pointer()
                            .a11y_button(format!("Remove photo {}", i + 1))
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_attachment(i, cx)))
                    })
                    .into_any_element()
            }).collect::<Vec<_>>();
            tray = tray.child(div().flex().flex_wrap().gap(px(8.)).children(chips));
        }
        if menu {
            tray = tray.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(outline_button("attach-camera", "Take Photo", true, t)
                        .on_click(cx.listener(|this, _, _, cx| this.pick_photo(Source::Camera, cx))))
                    .child(outline_button("attach-library", "Photo Library", true, t)
                        .on_click(cx.listener(|this, _, _, cx| this.pick_photo(Source::Library, cx)))),
            );
        }
        Some(tray)
    }
}

#[cfg(test)]
#[path = "attachment_tests.rs"]
mod tests;
