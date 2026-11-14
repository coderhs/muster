//! A small black strip around the display's camera notch that shows how many cards
//! are working and how many need the user. It is a native panel, not a webview, so it
//! costs almost nothing while idle. It is hidden when there is nothing to show, when
//! switched off in Settings, and on displays without a notch.

use std::sync::OnceLock;

/// What the strip shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub running: usize,
    pub needs_you: usize,
}

impl Counts {
    pub fn of<'a>(statuses: impl Iterator<Item = &'a str>) -> Counts {
        let mut c = Counts::default();
        for s in statuses {
            match s {
                "running" => c.running += 1,
                "needs_you" => c.needs_you += 1,
                _ => {}
            }
        }
        c
    }

    pub fn is_empty(self) -> bool {
        self.running == 0 && self.needs_you == 0
    }
}

static ON_CLICK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Installs the handler for a click on the strip.
pub fn init(on_click: impl Fn() + Send + Sync + 'static) {
    let _ = ON_CLICK.set(Box::new(on_click));
}

/// Whether some connected display has a notch. Main thread only.
pub fn available() -> bool {
    imp::available()
}

/// Shows, updates or hides the strip. Main thread only.
pub fn update(counts: Counts, enabled: bool) {
    imp::update(counts, enabled);
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Counts, ON_CLICK};
    use objc2::rc::Retained;
    use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSBezierPath, NSColor, NSEvent, NSFont, NSFontWeightSemibold, NSPanel, NSScreen,
        NSTextAlignment, NSTextField, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::cell::RefCell;

    /// Width of the strip on each side of the notch.
    const EAR: f64 = 46.0;
    const RADIUS: f64 = 9.0;
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

            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, _ev: &NSEvent) {
                if let Some(f) = ON_CLICK.get() {
                    f();
                }
            }
        }
    );

    struct Strip {
        panel: Retained<NSPanel>,
        running: Retained<NSTextField>,
        needs: Retained<NSTextField>,
        shown: Option<(Counts, NSRect)>,
    }

    thread_local! {
        static STRIP: RefCell<Option<Strip>> = const { RefCell::new(None) };
    }

    /// The strip's frame on the first display with a notch: the notch plus an ear on
    /// each side, as tall as the notch.
    fn notch_frame(mtm: MainThreadMarker) -> Option<NSRect> {
        for screen in NSScreen::screens(mtm).iter() {
            let top = screen.safeAreaInsets().top;
            if top <= 0.0 {
                continue;
            }
            let f = screen.frame();
            let (l, r) = (screen.auxiliaryTopLeftArea(), screen.auxiliaryTopRightArea());
            let notch = f.size.width - l.size.width - r.size.width;
            if notch <= 0.0 {
                continue;
            }
            let w = notch + 2.0 * EAR;
            let x = f.origin.x + l.size.width - EAR;
            return Some(NSRect::new(NSPoint::new(x, f.origin.y + f.size.height - top), NSSize::new(w, top)));
        }
        None
    }

    pub fn available() -> bool {
        MainThreadMarker::new().is_some_and(|mtm| notch_frame(mtm).is_some())
    }

    fn label(mtm: MainThreadMarker, color: &NSColor, align: NSTextAlignment) -> Retained<NSTextField> {
        let l = NSTextField::labelWithString(&NSString::new(), mtm);
        l.setTextColor(Some(color));
        l.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(11.0, unsafe { NSFontWeightSemibold })));
        l.setAlignment(align);
        l
    }

    fn build(mtm: MainThreadMarker, frame: NSRect) -> Strip {
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
        let running = label(mtm, &NSColor::systemGreenColor(), NSTextAlignment::Left);
        let needs = label(mtm, &NSColor::systemOrangeColor(), NSTextAlignment::Right);
        view.addSubview(&running);
        view.addSubview(&needs);
        panel.setContentView(Some(&view));
        Strip { panel, running, needs, shown: None }
    }

    pub fn update(counts: Counts, enabled: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        STRIP.with_borrow_mut(|slot| {
            let frame = if enabled && !counts.is_empty() { notch_frame(mtm) } else { None };
            let Some(frame) = frame else {
                if let Some(s) = slot.as_mut() {
                    s.panel.orderOut(None);
                    s.shown = None;
                }
                return;
            };
            let s = slot.get_or_insert_with(|| build(mtm, frame));
            if s.shown == Some((counts, frame)) {
                return;
            }
            let text = |n: usize, left: bool| match (n, left) {
                (0, _) => String::new(),
                (n, true) => format!("● {n}"),
                (n, false) => format!("{n} ●"),
            };
            s.running.setStringValue(&NSString::from_str(&text(counts.running, true)));
            s.needs.setStringValue(&NSString::from_str(&text(counts.needs_you, false)));
            s.running.setToolTip(Some(&NSString::from_str(&format!("{} working", counts.running))));
            s.needs.setToolTip(Some(&NSString::from_str(&format!("{} need you", counts.needs_you))));
            // Text sits in the ears, vertically centred, clear of the notch itself.
            let (h, pad, line) = (frame.size.height, 9.0, 15.0);
            let y = ((h - line) / 2.0).floor();
            s.running.setFrame(NSRect::new(NSPoint::new(pad, y), NSSize::new(EAR - pad, line)));
            s.needs.setFrame(NSRect::new(NSPoint::new(frame.size.width - EAR, y), NSSize::new(EAR - pad, line)));
            s.panel.setFrame_display(frame, true);
            s.panel.orderFrontRegardless();
            s.shown = Some((counts, frame));
        });
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Counts;
    pub fn available() -> bool {
        false
    }
    pub fn update(_: Counts, _: bool) {}
}

#[cfg(test)]
mod tests {
    use super::Counts;

    #[test]
    fn counts_working_and_waiting_cards() {
        let c = Counts::of(["running", "idle", "needs_you", "running", "done", "exited"].into_iter());
        assert_eq!(c, Counts { running: 2, needs_you: 1 });
        assert!(!c.is_empty());
        assert!(Counts::of(["idle", "done"].into_iter()).is_empty());
    }
}
