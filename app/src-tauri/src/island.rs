//! A small black strip right of the display's camera notch: how many cards are working
//! (orange), how many need the user (red), and how many have finished and not been
//! opened since (green; opening the card clears it). Under the pointer it grows
//! into a list of those cards by name; a click on a name opens that card.
//!
//! It is a native panel, not a webview, so it costs almost nothing while idle. It is
//! hidden when there is nothing to show, when switched off in Settings, and on
//! displays without a notch.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    NeedsYou,
    /// Finished and not opened since.
    Done,
    Working,
}

/// One card worth showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub kind: Kind,
}

/// At most this many names in the list; the rest are summed up in a last line.
pub const MAX_ROWS: usize = 8;

/// What a card with this status and unread flag shows as, if anything.
pub fn kind_of(status: &str, unread: bool) -> Option<Kind> {
    match (status, unread) {
        ("needs_you", _) => Some(Kind::NeedsYou),
        ("done", true) => Some(Kind::Done),
        ("running", _) => Some(Kind::Working),
        _ => None,
    }
}

/// Rows in the order shown: waiting longest first, then finished latest first, then
/// working. `cards` yields id, title, status, unread and the time of the last status
/// change.
pub fn rows<'a>(cards: impl Iterator<Item = (&'a str, &'a str, &'a str, bool, i64)>) -> Vec<Row> {
    let mut all: Vec<(Kind, i64, Row)> = cards
        .filter_map(|(id, title, status, unread, since)| {
            let kind = kind_of(status, unread)?;
            // Finished cards newest first; the others oldest first.
            let key = if kind == Kind::Done { -since } else { since };
            Some((kind, key, Row { id: id.into(), title: title.into(), kind }))
        })
        .collect();
    all.sort_by(|a, b| (a.0, a.1, &a.2.id).cmp(&(b.0, b.1, &b.2.id)));
    all.into_iter().map(|(_, _, r)| r).collect()
}

fn count(rows: &[Row], kind: Kind) -> usize {
    rows.iter().filter(|r| r.kind == kind).count()
}

static ON_CLICK: OnceLock<Box<dyn Fn(Option<String>) + Send + Sync>> = OnceLock::new();

/// Installs the handler for a click: on a card's name (its id), or on the strip itself.
pub fn init(on_click: impl Fn(Option<String>) + Send + Sync + 'static) {
    let _ = ON_CLICK.set(Box::new(on_click));
}

/// Whether some connected display has a notch. Main thread only.
pub fn available() -> bool {
    imp::available()
}

/// Shows, updates or hides the strip. Main thread only.
pub fn update(rows: Vec<Row>, enabled: bool) {
    imp::update(rows, enabled);
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{count, Kind, Row, MAX_ROWS, ON_CLICK};
    use objc2::rc::Retained;
    use objc2::{define_class, msg_send, AllocAnyThread, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSBezierPath, NSColor, NSEvent, NSFont, NSFontWeightMedium, NSFontWeightSemibold,
        NSLineBreakMode, NSPanel, NSScreen, NSTextAlignment, NSTextField, NSTrackingArea, NSTrackingAreaOptions,
        NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::cell::RefCell;

    /// Width of one count beside the notch, kept as narrow as a dot and a digit allow.
    const SLOT: f64 = 22.0;
    const RADIUS: f64 = 8.0;
    /// The list of names under the pointer.
    const LIST_W: f64 = 260.0;
    const ROW_H: f64 = 20.0;
    const LIST_PAD: f64 = 6.0;
    /// Above the menu bar (24) and status items (25).
    const LEVEL: isize = 27;

    define_class!(
        // SAFETY: NSView allows subclassing; no ivars, no Drop.
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        struct StripView;

        impl StripView {
            // Black, flush with the top edge, rounded at the bottom like the notch.
            #[unsafe(method(drawRect:))]
            fn draw_rect(&self, _dirty: NSRect) {
                let b = self.bounds();
                let r = NSRect::new(b.origin, NSSize::new(b.size.width, b.size.height + RADIUS));
                NSColor::blackColor().setFill();
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, RADIUS, RADIUS).fill();
            }

            #[unsafe(method(mouseEntered:))]
            fn mouse_entered(&self, _ev: &NSEvent) {
                set_expanded(true);
            }

            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _ev: &NSEvent) {
                set_expanded(false);
            }

            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, ev: &NSEvent) {
                let y = ev.locationInWindow().y;
                let card = STRIP.with_borrow(|s| s.as_ref().and_then(|s| s.card_at(y)));
                if let Some(f) = ON_CLICK.get() {
                    f(card);
                }
            }
        }
    );

    /// Where the notch is, in screen coordinates.
    #[derive(Clone, Copy, PartialEq)]
    struct Notch {
        x: f64,
        w: f64,
        /// Height of the notch (and of the menu bar beside it).
        h: f64,
        /// The top edge of the display.
        top: f64,
    }

    struct Strip {
        panel: Retained<NSPanel>,
        view: Retained<StripView>,
        running: Retained<NSTextField>,
        needs: Retained<NSTextField>,
        done: Retained<NSTextField>,
        names: Vec<Retained<NSTextField>>,
        rows: Vec<Row>,
        enabled: bool,
        expanded: bool,
        /// Height of the list part when expanded, for hit testing.
        list_h: f64,
    }

    thread_local! {
        static STRIP: RefCell<Option<Strip>> = const { RefCell::new(None) };
    }

    fn notch(mtm: MainThreadMarker) -> Option<Notch> {
        for screen in NSScreen::screens(mtm).iter() {
            let h = screen.safeAreaInsets().top;
            if h <= 0.0 {
                continue;
            }
            let f = screen.frame();
            let (l, r) = (screen.auxiliaryTopLeftArea(), screen.auxiliaryTopRightArea());
            let w = f.size.width - l.size.width - r.size.width;
            if w > 0.0 {
                return Some(Notch { x: f.origin.x + l.size.width, w, h, top: f.origin.y + f.size.height });
            }
        }
        None
    }

    pub fn available() -> bool {
        MainThreadMarker::new().is_some_and(|mtm| notch(mtm).is_some())
    }

    fn label(mtm: MainThreadMarker, color: &NSColor, size: f64, align: NSTextAlignment) -> Retained<NSTextField> {
        let l = NSTextField::labelWithString(&NSString::new(), mtm);
        l.setTextColor(Some(color));
        let weight = unsafe {
            if size < 11.0 {
                NSFontWeightSemibold
            } else {
                NSFontWeightMedium
            }
        };
        l.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(size, weight)));
        l.setAlignment(align);
        l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        l
    }

    fn color(kind: Kind) -> Retained<NSColor> {
        match kind {
            Kind::NeedsYou => NSColor::systemRedColor(),
            Kind::Done => NSColor::systemGreenColor(),
            Kind::Working => NSColor::systemOrangeColor(),
        }
    }

    fn build(mtm: MainThreadMarker) -> Strip {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(10.0, 10.0));
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setLevel(LEVEL);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setHidesOnDeactivate(false);
        // On every Space and over full-screen apps, and never in the window cycle.
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        let view: Retained<StripView> = unsafe { msg_send![StripView::alloc(mtm), initWithFrame: frame] };
        // Follows the view as it grows and shrinks; works while another app is in front.
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                frame,
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&tracking);
        let running = label(mtm, &color(Kind::Working), 10.0, NSTextAlignment::Center);
        let needs = label(mtm, &color(Kind::NeedsYou), 10.0, NSTextAlignment::Center);
        let done = label(mtm, &color(Kind::Done), 10.0, NSTextAlignment::Center);
        view.addSubview(&running);
        view.addSubview(&needs);
        view.addSubview(&done);
        panel.setContentView(Some(&view));
        Strip {
            panel,
            view,
            running,
            needs,
            done,
            names: Vec::new(),
            rows: Vec::new(),
            enabled: false,
            expanded: false,
            list_h: 0.0,
        }
    }

    impl Strip {
        /// The card whose name is at window height `y`, if the list is open there.
        fn card_at(&self, y: f64) -> Option<String> {
            if !self.expanded || y >= self.list_h {
                return None;
            }
            let i = ((self.list_h - LIST_PAD / 2.0 - y) / ROW_H).floor();
            if i < 0.0 {
                return None;
            }
            self.rows.get(i as usize).filter(|_| (i as usize) < MAX_ROWS).map(|r| r.id.clone())
        }

        fn layout(&mut self, mtm: MainThreadMarker) {
            let notch = if self.enabled && !self.rows.is_empty() { notch(mtm) } else { None };
            let Some(n) = notch else {
                self.expanded = false;
                self.panel.orderOut(None);
                return;
            };
            let (working, needs, done) =
                (count(&self.rows, Kind::Working), count(&self.rows, Kind::NeedsYou), count(&self.rows, Kind::Done));
            let slots = |k: usize| if k > 0 { SLOT } else { 0.0 };
            // Everything sits right of the notch, so one card is seen changing in
            // place: working, then finished.
            let (left, right) = (0.0, slots(working) + slots(needs) + slots(done));

            // Collapsed: the notch plus only the counts that have something to say.
            let (mut x0, mut x1) = (n.x - left, n.x + n.w + right);
            let shown = self.rows.len().min(MAX_ROWS);
            let more = self.rows.len() - shown;
            self.list_h = if self.expanded { (shown + usize::from(more > 0)) as f64 * ROW_H + LIST_PAD } else { 0.0 };
            if self.expanded {
                let mid = n.x + n.w / 2.0;
                x0 = x0.min(mid - LIST_W / 2.0);
                x1 = x1.max(mid + LIST_W / 2.0);
            }
            let h = n.h + self.list_h;
            let frame = NSRect::new(NSPoint::new(x0, n.top - h), NSSize::new(x1 - x0, h));

            let set = |l: &NSTextField, k: usize, text: String, x: f64| {
                l.setStringValue(&NSString::from_str(if k > 0 { &text } else { "" }));
                let line = 13.0;
                let y = self.list_h + ((n.h - line) / 2.0).floor();
                l.setFrame(NSRect::new(NSPoint::new(x - x0, y), NSSize::new(SLOT, line)));
            };
            set(&self.running, working, format!("●{working}"), n.x + n.w);
            set(&self.needs, needs, format!("!{needs}"), n.x + n.w + slots(working));
            set(&self.done, done, format!("✓{done}"), n.x + n.w + slots(working) + slots(needs));

            for l in self.names.drain(..) {
                l.removeFromSuperview();
            }
            if self.expanded {
                let mut line = |i: usize, text: String, c: &NSColor| {
                    let l = label(mtm, c, 12.0, NSTextAlignment::Left);
                    l.setStringValue(&NSString::from_str(&text));
                    let y = self.list_h - LIST_PAD / 2.0 - (i + 1) as f64 * ROW_H + 2.0;
                    l.setFrame(NSRect::new(NSPoint::new(12.0, y), NSSize::new(x1 - x0 - 24.0, ROW_H - 4.0)));
                    self.view.addSubview(&l);
                    self.names.push(l);
                };
                for (i, r) in self.rows.iter().take(shown).enumerate() {
                    let mark = match r.kind {
                        Kind::NeedsYou => "!  Needs you",
                        Kind::Done => "✓  Finished",
                        Kind::Working => "●  Working",
                    };
                    line(i, format!("{mark} · {}", r.title), &color(r.kind));
                }
                if more > 0 {
                    line(shown, format!("and {more} more"), &NSColor::grayColor());
                }
            }
            self.panel.setFrame_display(frame, true);
            self.view.setNeedsDisplay(true);
            self.panel.orderFrontRegardless();
        }
    }

    fn set_expanded(on: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        STRIP.with(|cell| {
            // Never re-entered in practice; skip rather than panic if it ever is.
            let Ok(mut slot) = cell.try_borrow_mut() else { return };
            if let Some(s) = slot.as_mut() {
                if s.expanded != on {
                    s.expanded = on;
                    s.layout(mtm);
                }
            }
        });
    }

    pub fn update(rows: Vec<Row>, enabled: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        STRIP.with(|cell| {
            let Ok(mut slot) = cell.try_borrow_mut() else { return };
            if slot.is_none() && (!enabled || rows.is_empty()) {
                return;
            }
            let s = slot.get_or_insert_with(|| build(mtm));
            if s.rows == rows && s.enabled == enabled {
                return;
            }
            s.rows = rows;
            s.enabled = enabled;
            s.layout(mtm);
        });
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Row;
    pub fn available() -> bool {
        false
    }
    pub fn update(_: Vec<Row>, _: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_waiting_then_finished_then_working() {
        let cards = [
            ("a", "Build", "running", false, 50),
            ("b", "Idle one", "idle", false, 10),
            ("c", "Asked late", "needs_you", true, 40),
            ("d", "Asked early", "needs_you", true, 20),
            ("e", "Done early", "done", true, 30),
            ("f", "Done late", "done", true, 60),
            ("g", "Done and read", "done", false, 70),
            ("h", "Gone", "exited", false, 80),
        ];
        let got = rows(cards.into_iter());
        let ids: Vec<&str> = got.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["d", "c", "f", "e", "a"]);
        assert_eq!(count(&got, Kind::NeedsYou), 2);
        assert_eq!(count(&got, Kind::Done), 2);
        assert_eq!(count(&got, Kind::Working), 1);
    }

    #[test]
    fn a_finished_card_shows_only_until_opened() {
        assert_eq!(kind_of("done", true), Some(Kind::Done));
        assert_eq!(kind_of("done", false), None);
        assert_eq!(kind_of("idle", true), None);
    }
}
