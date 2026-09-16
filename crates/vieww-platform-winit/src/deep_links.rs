//! [`DeepLinks`] over the platform's own link delivery.
//!
//! # The two halves, and why they are separate methods
//!
//! A link either **launched** the process or **arrived at one already running**,
//! and the platforms deliver them through completely different paths — an
//! `Intent` on the activity versus `onNewIntent`, a launch option versus
//! `application(_:open:)`. Collapsing them into one callback is the standard
//! way to build an app that opens the right screen from cold and ignores the
//! link when it is already in the foreground, which is the harder bug to notice
//! because the developer testing it always has the app open.
//!
//! # Links arriving at a running app, on Android
//!
//! `android-activity` does not surface `onNewIntent`, so there is no callback to
//! subscribe to. There are two ways in and **both are wired**, because they fail
//! in different situations and an application should not have to know which one
//! it is relying on:
//!
//! 1. **A native entry point.** [`vieww_deep_link_deliver`] is an `extern "C"`
//!    symbol any native code can call, and
//!    `Java_dev_vieww_DeepLinks_deliver` is the JNI-named wrapper a Java shim
//!    calls. Exact, immediate, and needs six lines of Java in the
//!    application's own activity — given below.
//! 2. **Polling the activity's intent.** Every [`take_pending`](DeepLinks::take_pending)
//!    re-reads `getIntent().getDataString()` and reports it if it changed. This
//!    needs no Java at all and works for any activity that calls `setIntent` in
//!    its `onNewIntent`, which the platform's own subclasses do. A bare
//!    `NativeActivity` does not, which is exactly why route 1 exists.
//!
//! The Java shim, for an application that wants the exact route:
//!
//! ```java
//! public final class DeepLinks {
//!     public static native void deliver(String url);   // implemented here
//! }
//!
//! // in your Activity:
//! @Override protected void onNewIntent(Intent intent) {
//!     super.onNewIntent(intent);
//!     setIntent(intent);                                // makes route 2 work too
//!     if (intent.getDataString() != null) {
//!         DeepLinks.deliver(intent.getDataString());    // route 1
//!     }
//! }
//! ```
//!
//! Calling both is harmless: a link delivered twice in one frame is
//! deduplicated against the last one seen.

use std::cell::RefCell;

use vieww_foundation::{DeepLink, DeepLinks, ServiceError};

/// Links handed in from outside any `PlatformDeepLinks`.
///
/// # Why this is global
///
/// The JNI entry point is a bare `extern "C"` function. It is called by the
/// platform, on the platform's schedule, with no argument that could carry a
/// handle to anything — so there is nowhere to put a link except somewhere the
/// process can find without being told. That is a real constraint of the C ABI
/// rather than a shortcut.
///
/// A `Mutex` rather than a `RefCell` because that call arrives on the Android
/// main thread while the UI thread may be mid-frame, and this is the one place
/// in `vieww` where two threads genuinely meet.
static DELIVERED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Hand a link to the running application.
///
/// The stable C entry point — callable from a JNI shim, from an iOS
/// `application(_:open:)` bridge, or from a test. Safe to call from any thread.
///
/// # Safety
///
/// `url` must be a valid NUL-terminated C string, or null. A null pointer is
/// ignored rather than treated as an error, because the platform passes one for
/// an intent that carries no data.
#[no_mangle]
pub unsafe extern "C" fn vieww_deep_link_deliver(url: *const std::ffi::c_char) {
    if url.is_null() {
        return;
    }
    // SAFETY: the caller's contract, checked for null above.
    let Ok(url) = (unsafe { std::ffi::CStr::from_ptr(url) }).to_str() else {
        // Not UTF-8. A URL that is not is not one this can route, and dropping
        // it is better than delivering mojibake to a router.
        return;
    };
    push_delivered(url.to_owned());
}

fn push_delivered(url: String) {
    if let Ok(mut queue) = DELIVERED.lock() {
        queue.push(url);
    }
}

/// Drain whatever has been delivered from outside.
fn take_delivered() -> Vec<String> {
    DELIVERED
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

/// The JNI-named wrapper for [`vieww_deep_link_deliver`].
///
/// Bound to `dev.vieww.DeepLinks.deliver(String)`. The class name is a contract
/// with the application's Java shim; see this module's docs for the six lines
/// that satisfy it.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_dev_vieww_DeepLinks_deliver<'local>(
    mut env: jni::EnvUnowned<'local>,
    _class: jni::objects::JClass<'local>,
    url: jni::objects::JString<'local>,
) {
    // `EnvUnowned` rather than `Env`, because `jni` 0.22 split the old
    // `JNIEnv` in two and only this half is FFI-safe — it is the caller's
    // frame, not an attachment this code owns. `with_env` borrows it to
    // produce the `Env` that actually carries the methods.
    //
    // The `catch_unwind` `with_env` wraps the closure in is load-bearing
    // rather than tidy: unwinding across a native method boundary does not
    // return an error to Java, it aborts the process.
    let outcome = env.with_env(|env| -> Result<Option<String>, jni::errors::Error> {
        if url.is_null() {
            // The shim documented above skips a null, but a native method
            // does not get to assume its caller is the documented one.
            return Ok(None);
        }
        Ok(Some(url.try_to_string(env)?))
    });

    // `into_outcome` rather than `resolve::<Policy>()`. Resolving wants an
    // `ErrorPolicy` saying which Java exception to throw, and this method has
    // no opinion to express: a link it cannot read is a link it drops, exactly
    // as `vieww_deep_link_deliver` drops one that is not UTF-8. Throwing back
    // into an application's `onNewIntent` over a malformed URL would turn a
    // missed navigation into a crash.
    if let jni::Outcome::Ok(Some(url)) = outcome.into_outcome() {
        push_delivered(url);
    }
}

/// [`DeepLinks`] for the host platform.
#[derive(Debug, Default)]
pub struct PlatformDeepLinks {
    initial: RefCell<Option<DeepLink>>,
    pending: RefCell<Vec<DeepLink>>,
    /// The activity's intent as of the last poll.
    ///
    /// Both delivery routes feed one queue, so a link that arrives through the
    /// native entry point *and* through the intent poll would be routed twice —
    /// which navigates, and then navigates again on top of itself. This is what
    /// makes the second one a no-op.
    last_seen: RefCell<Option<String>>,
    /// The activity, for polling its intent. `None` off Android.
    #[cfg(target_os = "android")]
    android: Option<crate::AndroidApp>,
}

impl PlatformDeepLinks {
    /// No launch link.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// With a known launch link.
    #[must_use]
    pub fn with_initial(link: DeepLink) -> Self {
        // The launch link is recorded as already seen, so the intent poll does
        // not report it a second time as though it had just arrived.
        let raw = link.raw.clone();
        Self {
            initial: RefCell::new(Some(link)),
            last_seen: RefCell::new(Some(raw)),
            ..Self::default()
        }
    }

    /// Reading the launch link from the command line.
    ///
    /// The first argument that looks like a URL. This is how a desktop delivers
    /// a registered scheme, and it is also the only way to exercise deep-link
    /// routing without a phone — which matters more than the desktop case
    /// itself, since desktop is a development harness here.
    #[must_use]
    pub fn from_command_line() -> Self {
        match std::env::args()
            .skip(1)
            .find(|argument| argument.contains("://"))
        {
            Some(url) => Self::with_initial(DeepLink::new(url)),
            None => Self::none(),
        }
    }

    /// Reading the launch link from the Android activity's intent.
    ///
    /// The activity is kept, because it is also what
    /// [`take_pending`](DeepLinks::take_pending) polls and what
    /// [`open`](DeepLinks::open) needs.
    #[cfg(target_os = "android")]
    #[must_use]
    pub fn from_android(android: &crate::AndroidApp) -> Self {
        let mut links = match android_launch_link(android) {
            Some(link) => Self::with_initial(link),
            None => Self::none(),
        };
        links.android = Some(android.clone());
        links
    }

    /// Hand a link to the application as though the platform had delivered it.
    ///
    /// The entry point an application's own activity shim calls from
    /// `onNewIntent`, and what a test uses to drive routing with no platform at
    /// all.
    pub fn deliver(&self, link: DeepLink) {
        self.enqueue(link.raw);
    }

    /// Queue a URL unless it is the one already seen.
    ///
    /// Both delivery routes end here, which is what makes calling both of them
    /// — the Java shim *and* the intent poll — harmless.
    fn enqueue(&self, url: String) {
        if self.last_seen.borrow().as_deref() == Some(url.as_str()) {
            return;
        }
        *self.last_seen.borrow_mut() = Some(url.clone());
        self.pending.borrow_mut().push(DeepLink::new(url));
    }

    /// Take in anything the native entry point was handed, and poll the
    /// platform for anything it did not tell us about.
    fn collect(&self) {
        for url in take_delivered() {
            self.enqueue(url);
        }
        #[cfg(target_os = "android")]
        if let Some(android) = &self.android {
            if let Some(link) = android_launch_link(android) {
                self.enqueue(link.raw);
            }
        }
    }
}

impl DeepLinks for PlatformDeepLinks {
    fn take_initial(&self) -> Option<DeepLink> {
        self.initial.borrow_mut().take()
    }

    fn take_pending(&self) -> Option<DeepLink> {
        // Polled here rather than pushed from a callback, for the reason the
        // trait documents: the platform delivers on a thread and at a time that
        // have nothing to do with the UI, and a signal may only be written on
        // the UI thread. This runs during a frame, where writing is allowed.
        self.collect();

        let mut pending = self.pending.borrow_mut();
        if pending.is_empty() {
            return None;
        }
        Some(pending.remove(0))
    }

    fn open(&self, link: &DeepLink) -> Result<(), ServiceError> {
        #[cfg(target_os = "android")]
        {
            return match &self.android {
                Some(android) => android_open(android, &link.raw),
                None => Err(ServiceError::failed(
                    "opening a link needs the activity — build with \
                     PlatformDeepLinks::from_android",
                )),
            };
        }
        #[cfg(not(target_os = "android"))]
        open_externally(&link.raw)
    }
}

/// `startActivity(new Intent(ACTION_VIEW, Uri.parse(url)))`.
///
/// The `FLAG_ACTIVITY_NEW_TASK` is not optional: an activity started from a
/// context that is not itself an activity throws, and while this one *is*, the
/// flag is what keeps the opened page in its own task rather than on top of the
/// application's own back stack — where the back button would return to it.
#[cfg(target_os = "android")]
fn android_open(android: &crate::AndroidApp, url: &str) -> Result<(), ServiceError> {
    use jni::objects::{JObject, JValue};
    use jni::{jni_sig, jni_str};

    /// `Intent.FLAG_ACTIVITY_NEW_TASK`.
    const NEW_TASK: i32 = 0x1000_0000;

    let vm = unsafe { jni::JavaVM::from_raw(android.vm_as_ptr().cast()) };
    let activity_ptr = android.activity_as_ptr();

    let opened = vm.attach_current_thread(|env| -> Result<(), jni::errors::Error> {
        let activity = unsafe { JObject::from_raw(env, activity_ptr.cast()) };

        let url = env.new_string(url)?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::Object(&url)],
            )?
            .l()?;

        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            jni_str!("android/content/Intent"),
            jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[JValue::Object(&action), JValue::Object(&uri)],
        )?;
        env.call_method(
            &intent,
            jni_str!("addFlags"),
            jni_sig!("(I)Landroid/content/Intent;"),
            &[JValue::Int(NEW_TASK)],
        )?;

        env.call_method(
            &activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    });

    opened.map_err(|error| {
        // `ActivityNotFoundException` is the ordinary case here — a phone with
        // nothing registered for the scheme — so this is a failure rather than
        // an unsupported capability.
        ServiceError::failed(format!("opening {url}: {error}"))
    })
}

#[cfg(not(target_os = "android"))]
fn open_externally(url: &str) -> Result<(), ServiceError> {
    // iOS has no `Command`, and a sandboxed app cannot spawn anything anyway.
    if cfg!(target_os = "ios") {
        return Err(ServiceError::unsupported("DeepLinks::open"));
    }

    let (program, first) = if cfg!(target_os = "macos") {
        ("open", None)
    } else if cfg!(target_os = "windows") {
        // `start` is a shell builtin, and its first argument is the window
        // title — which is why the empty string is there and is not a mistake.
        ("cmd", Some(vec!["/C", "start", ""]))
    } else {
        ("xdg-open", None)
    };

    let mut command = std::process::Command::new(program);
    if let Some(arguments) = first {
        command.args(arguments);
    }
    command
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| ServiceError::failed(format!("opening {url}: {error}")))
}

/// The URL the activity was launched with, if any.
///
/// `getIntent().getDataString()`, which is `null` for an ordinary launch from
/// the home screen and a URL when the app was opened by a link.
#[cfg(target_os = "android")]
fn android_launch_link(android: &crate::AndroidApp) -> Option<DeepLink> {
    use jni::objects::{JObject, JString};
    use jni::{jni_sig, jni_str};

    let vm = unsafe { jni::JavaVM::from_raw(android.vm_as_ptr().cast()) };
    let activity_ptr = android.activity_as_ptr();

    let queried = vm.attach_current_thread(|env| -> Result<Option<String>, jni::errors::Error> {
        let activity = unsafe { JObject::from_raw(env, activity_ptr.cast()) };

        let intent = env
            .call_method(
                &activity,
                jni_str!("getIntent"),
                jni_sig!("()Landroid/content/Intent;"),
                &[],
            )?
            .l()?;
        if intent.is_null() {
            return Ok(None);
        }

        let data = env
            .call_method(
                &intent,
                jni_str!("getDataString"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        if data.is_null() {
            return Ok(None);
        }

        // Two fallible steps, spelled out. `JString::from` used to make the
        // first infallible and 0.22 removed it, which is the better API: a
        // cast is a real `IsInstanceOf` against a value the platform handed
        // over, and `getDataString` is only documented to return a `String`.
        // Reading the characters is then a separate call that can fail on its
        // own, so collapsing the two would report one as the other.
        let url = env.cast_local::<JString>(data)?;
        Ok(Some(url.try_to_string(env)?))
    });

    queried.ok().flatten().map(DeepLink::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises every test that touches the process-wide delivery queue, and
    /// empties it on the way in.
    ///
    /// `DELIVERED` is a `static` because the C entry point has nowhere else to
    /// put a link — see its own docs. That makes it shared mutable state across
    /// a test binary whose tests run in parallel, so any test calling
    /// `take_pending` can drain a link another test just pushed. One lock, held
    /// by all of them, rather than a flake that reproduces once a fortnight.
    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = take_delivered();
        guard
    }

    #[test]
    fn a_launch_link_is_delivered_once_and_then_gone() {
        // A router that re-reads it every rebuild navigates back to it whenever
        // anything else changes, which is why this is `take` and not `get`.
        let links = PlatformDeepLinks::with_initial(DeepLink::new("myapp://orders/17"));
        // Bound rather than chained: `path` borrows the link it came from, so
        // mapping it out of a temporary leaves the borrow with nothing to point
        // at.
        let path = links.take_initial().map(|link| link.path().to_owned());
        assert_eq!(path.as_deref(), Some("/orders/17"));
        assert!(links.take_initial().is_none());
    }

    #[test]
    fn links_delivered_while_running_come_back_in_order() {
        let _guard = exclusive();
        let links = PlatformDeepLinks::none();
        links.deliver(DeepLink::new("myapp://a"));
        links.deliver(DeepLink::new("myapp://b"));

        assert_eq!(links.take_pending().expect("a").raw, "myapp://a");
        assert_eq!(links.take_pending().expect("b").raw, "myapp://b");
        assert!(links.take_pending().is_none());
    }

    #[test]
    fn a_launch_link_is_not_also_a_pending_one() {
        let _guard = exclusive();
        // Delivering it through both paths is how an app navigates twice and
        // ends up with the target screen on the stack under itself.
        let links = PlatformDeepLinks::with_initial(DeepLink::new("myapp://x"));
        assert!(links.take_pending().is_none());
    }

    #[test]
    fn no_launch_link_is_the_ordinary_case() {
        assert!(PlatformDeepLinks::none().take_initial().is_none());
    }

    #[test]
    fn the_same_link_twice_in_a_row_is_delivered_once() {
        let _guard = exclusive();
        // Both routes are wired on Android — the Java shim and the intent poll
        // — so an application that has the shim gets every link twice. This is
        // what makes having both harmless rather than a double navigation.
        let links = PlatformDeepLinks::none();
        links.deliver(DeepLink::new("myapp://orders/17"));
        links.deliver(DeepLink::new("myapp://orders/17"));

        assert_eq!(
            links.take_pending().expect("the link").raw,
            "myapp://orders/17"
        );
        assert!(links.take_pending().is_none());
    }

    #[test]
    fn the_same_link_again_after_a_different_one_is_a_new_arrival() {
        let _guard = exclusive();
        // Deduplication is against the *last* link, not against everything ever
        // seen — tapping the same notification twice with something in between
        // is two real navigations.
        let links = PlatformDeepLinks::none();
        links.deliver(DeepLink::new("myapp://a"));
        links.deliver(DeepLink::new("myapp://b"));
        links.deliver(DeepLink::new("myapp://a"));

        let seen: Vec<String> = std::iter::from_fn(|| links.take_pending())
            .map(|link| link.raw)
            .collect();
        assert_eq!(seen, ["myapp://a", "myapp://b", "myapp://a"]);
    }

    #[test]
    fn the_native_entry_point_reaches_a_running_app() {
        let _guard = exclusive();
        // The C ABI route, which is what the JNI shim and an iOS bridge both
        // funnel into. Exercised here rather than only on a device, because
        // this half needs no platform at all.
        let url = std::ffi::CString::new("myapp://from-native").expect("a C string");
        // SAFETY: a valid NUL-terminated string that outlives the call.
        unsafe { vieww_deep_link_deliver(url.as_ptr()) };

        let links = PlatformDeepLinks::none();
        assert_eq!(
            links.take_pending().map(|link| link.raw),
            Some("myapp://from-native".to_owned())
        );
    }

    #[test]
    fn a_null_url_from_the_platform_is_ignored_rather_than_crashing() {
        let _guard = exclusive();
        // The platform passes one for an intent that carries no data, which is
        // every ordinary launch from the home screen.
        // SAFETY: null is explicitly part of this function's contract.
        unsafe { vieww_deep_link_deliver(std::ptr::null()) };
        assert!(PlatformDeepLinks::none().take_pending().is_none());
    }
}
