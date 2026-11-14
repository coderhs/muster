//! macOS notifications through the UserNotifications framework.
//!
//! Permission is requested the first time a notification is needed. A click on a
//! notification calls the handler installed with `init` with the card id. The framework
//! only works from inside an app bundle (Muster.app); a bare binary has no bundle
//! identifier, so there `status()` reports `Unavailable` and nothing is posted.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not running from an app bundle.
    Unavailable,
    /// Permission not asked yet (asked on first need).
    Unknown,
    Requesting,
    Granted,
    Denied,
}

static STATUS: AtomicU8 = AtomicU8::new(0);
static ON_CLICK: OnceLock<Box<dyn Fn(String) + Send + Sync>> = OnceLock::new();

fn set(s: Status) {
    STATUS.store(s as u8 + 1, Ordering::SeqCst);
}

pub fn status() -> Status {
    match STATUS.load(Ordering::SeqCst) {
        1 => Status::Unavailable,
        2 => Status::Unknown,
        3 => Status::Requesting,
        4 => Status::Granted,
        5 => Status::Denied,
        _ => Status::Unavailable,
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, AllocAnyThread};
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions,
        UNNotificationRequest, UNNotificationResponse, UNNotificationSound, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; no ivars, no Drop.
        #[unsafe(super(NSObject))]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                completion: &block2::DynBlock<dyn Fn()>,
            ) {
                let id = response.notification().request().identifier().to_string();
                if let Some(card) = id.strip_prefix("card:").and_then(|r| r.split(':').next()) {
                    if let Some(f) = ON_CLICK.get() {
                        f(card.to_string());
                    }
                }
                completion.call(());
            }

            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                // Shown even when Muster is the active app (e.g. its window is hidden).
                completion.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List
                    | UNNotificationPresentationOptions::Sound,));
            }
        }
    );

    fn in_bundle() -> bool {
        NSBundle::mainBundle().bundleIdentifier().is_some()
    }

    /// Installs the delegate. Call once, on the main thread.
    pub fn init(on_click: impl Fn(String) + Send + Sync + 'static) {
        let _ = ON_CLICK.set(Box::new(on_click));
        if !in_bundle() {
            set(Status::Unavailable);
            return;
        }
        set(Status::Unknown);
        let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
        let center = UNUserNotificationCenter::currentNotificationCenter();
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The center holds its delegate weakly; keep ours for the life of the app.
        std::mem::forget(delegate);
    }

    fn deliver(title: &str, subtitle: &str, body: &str, card_id: &str) {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        if !subtitle.is_empty() {
            content.setSubtitle(&NSString::from_str(subtitle));
        }
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        content.setThreadIdentifier(&NSString::from_str(card_id));
        let ident = format!(
            "card:{card_id}:{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
        );
        let req =
            UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&ident), &content, None);
        let done = RcBlock::new(|err: *mut NSError| {
            if !err.is_null() {
                eprintln!("muster: notification not delivered: {}", unsafe { &*err }.localizedDescription());
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&req, Some(&done));
    }

    /// Posts a notification, asking for permission first if it has not been asked yet.
    pub fn post(title: String, subtitle: String, body: String, card_id: String) {
        match status() {
            Status::Granted => deliver(&title, &subtitle, &body, &card_id),
            Status::Unknown => {
                set(Status::Requesting);
                let block = RcBlock::new(move |granted: Bool, err: *mut NSError| {
                    if granted.as_bool() {
                        set(Status::Granted);
                        deliver(&title, &subtitle, &body, &card_id);
                    } else {
                        set(Status::Denied);
                        if !err.is_null() {
                            eprintln!(
                                "muster: notification permission error: {}",
                                unsafe { &*err }.localizedDescription()
                            );
                        }
                    }
                });
                UNUserNotificationCenter::currentNotificationCenter()
                    .requestAuthorizationWithOptions_completionHandler(
                        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge,
                        &block,
                    );
            }
            // Requesting: drop this one; Denied/Unavailable: nothing to do.
            _ => {}
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{init, post};

#[cfg(not(target_os = "macos"))]
pub fn init(_on_click: impl Fn(String) + Send + Sync + 'static) {
    set(Status::Unavailable);
}

#[cfg(not(target_os = "macos"))]
pub fn post(_title: String, _subtitle: String, _body: String, _card_id: String) {}
