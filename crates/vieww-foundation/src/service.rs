//! The seam platform capabilities arrive through.
//!
//! Storage, deep links, the camera, geolocation, push, biometrics: everything a
//! real application needs from the device and that a UI framework has no
//! business implementing twice. This module is not any of them. It is the shape
//! they plug into, and it exists **before** the first one on purpose — the
//! alternative is that the first service is written ad hoc, the second copies
//! it, and the third has to retrofit both.
//!
//! # The rule that shapes everything here
//!
//! **A third party must be able to add a service without editing this crate.**
//!
//! That is the whole design constraint, and it is what a registry of named
//! fields would have failed. A framework's real moat is not its widgets, it is that
//! forty thousand packages can reach the platform without forking the engine. A
//! seam that only vieww itself can extend would look like this one and be worth
//! very little.
//!
//! So: a service is **any `'static` trait you like**, registered by its own type
//! and looked up by it. vieww defines [`Storage`] and [`DeepLinks`] because it
//! needs them, not because the mechanism knows about them.
//!
//! ```
//! use std::rc::Rc;
//! use vieww_foundation::service::{ServiceError, Services};
//!
//! // Someone else's crate, with no vieww change of any kind.
//! trait Torch: 'static {
//!     fn set(&self, on: bool) -> Result<(), ServiceError>;
//! }
//!
//! struct NoTorch;
//! impl Torch for NoTorch {
//!     fn set(&self, _on: bool) -> Result<(), ServiceError> {
//!         Err(ServiceError::unsupported("Torch"))
//!     }
//! }
//!
//! let mut services = Services::new();
//! services.provide::<dyn Torch>(Rc::new(NoTorch));
//!
//! assert!(services.get::<dyn Torch>().is_some());
//! ```
//!
//! # Threading, and why these are not `Send`
//!
//! Services are held as [`Rc`] and used from the UI thread, for the reason
//! `Signal` is (`docs/DESIGN.md` §1). A service whose *implementation* needs a
//! background thread does that internally and reports back through
//! [`task`](crate::task) — which is why the fast, sync-on-every-platform
//! capabilities here return values directly and the slow ones are expected to
//! return a [`Task`](crate::task::Task). Making every call async so that the
//! interesting ones can be would tax the boring ones forever.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::TargetPlatform;

/// Why a platform capability did not work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    /// This platform does not have this capability at all.
    ///
    /// Distinct from a failure, and the distinction matters to the caller: an
    /// application can reasonably hide a button for something unsupported, and
    /// cannot reasonably hide it because one call failed.
    Unsupported {
        service: &'static str,
        platform: Option<TargetPlatform>,
    },
    /// The user, or the operating system, refused.
    ///
    /// Also distinct: retrying is pointless until something changes outside the
    /// application, and the usual correct response is to explain rather than to
    /// try again.
    Denied(String),
    /// It exists, it was permitted, and it went wrong anyway.
    Failed(String),
}

impl ServiceError {
    /// Not available on this platform.
    #[must_use]
    pub const fn unsupported(service: &'static str) -> Self {
        Self::Unsupported {
            service,
            platform: None,
        }
    }

    /// Not available on this specific platform.
    #[must_use]
    pub const fn unsupported_on(service: &'static str, platform: TargetPlatform) -> Self {
        Self::Unsupported {
            service,
            platform: Some(platform),
        }
    }

    #[must_use]
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    #[must_use]
    pub fn denied(message: impl Into<String>) -> Self {
        Self::Denied(message.into())
    }

    /// `true` when retrying could not possibly help.
    #[must_use]
    pub const fn is_permanent(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported {
                service,
                platform: Some(platform),
            } => write!(f, "{service} is not available on {platform:?}"),
            Self::Unsupported {
                service,
                platform: None,
            } => write!(f, "{service} is not available on this platform"),
            Self::Denied(message) => write!(f, "refused: {message}"),
            Self::Failed(message) => write!(f, "failed: {message}"),
        }
    }
}

impl std::error::Error for ServiceError {}

/// The capabilities available to an application.
///
/// Handed down the tree, so any widget can ask for one during `build`. Lookup is
/// by the trait's own type, so nothing here has to know what services exist —
/// see the module docs for why that is the point rather than a detail.
#[derive(Default)]
pub struct Services {
    /// Keyed by `TypeId::of::<dyn Trait>()`, holding a boxed `Rc<dyn Trait>`.
    ///
    /// The double indirection is what buys the open set. `Rc<dyn Trait>` is
    /// itself `Sized + 'static`, so it can be boxed as `dyn Any` and downcast
    /// back out — which a bare `Rc<dyn Trait>` stored as `Rc<dyn Any>` cannot,
    /// because the trait object's vtable is lost on the way in.
    entries: HashMap<TypeId, Box<dyn Any>>,
}

impl fmt::Debug for Services {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The services themselves are trait objects with nothing printable in
        // common, so the count is the honest summary.
        f.debug_struct("Services")
            .field("registered", &self.entries.len())
            .finish()
    }
}

impl Services {
    /// Nothing available. Every lookup returns `None`.
    ///
    /// The correct state for a test, and for a headless harness: an application
    /// that degrades when a capability is missing is one that will survive
    /// meeting a platform that lacks it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Make a capability available.
    ///
    /// `S` is the *trait*, written as `dyn Trait`, and it is what callers look
    /// the service up by. Registering twice replaces.
    ///
    /// ```
    /// # use std::rc::Rc;
    /// # use vieww_foundation::service::{MemoryStorage, Services, Storage};
    /// let mut services = Services::new();
    /// services.provide::<dyn Storage>(Rc::new(MemoryStorage::new()));
    /// ```
    pub fn provide<S: ?Sized + 'static>(&mut self, service: Rc<S>) {
        self.entries.insert(TypeId::of::<S>(), Box::new(service));
    }

    /// The capability, if this platform has it.
    #[must_use]
    pub fn get<S: ?Sized + 'static>(&self) -> Option<Rc<S>> {
        self.entries
            .get(&TypeId::of::<S>())
            .and_then(|entry| entry.downcast_ref::<Rc<S>>())
            .cloned()
    }

    /// The capability, or an error naming what was missing.
    ///
    /// For the common shape where a caller is going to return
    /// `Result<_, ServiceError>` anyway and would otherwise write the same
    /// `ok_or_else` at every call site.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] when nothing is registered for `S`.
    pub fn require<S: ?Sized + 'static>(&self, name: &'static str) -> Result<Rc<S>, ServiceError> {
        self.get::<S>().ok_or(ServiceError::unsupported(name))
    }

    /// `true` if anything is registered for `S`.
    #[must_use]
    pub fn has<S: ?Sized + 'static>(&self) -> bool {
        self.entries.contains_key(&TypeId::of::<S>())
    }

    /// How many capabilities are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A [`Services`] several widgets can hold at once.
///
/// The type that actually travels down the tree, and the reason [`Services`]
/// itself is deliberately **not** `Clone`: a registry is built once at startup
/// and then frozen into one of these. Copying it would be copying handles to
/// capabilities, which reads as harmless and is how two widgets end up writing
/// to what they each believe is the only store.
#[derive(Debug, Clone, Default)]
pub struct SharedServices(Rc<Services>);

impl SharedServices {
    #[must_use]
    pub fn new(services: Services) -> Self {
        Self(Rc::new(services))
    }

    /// Nothing available.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Services::new())
    }

    /// The capability, if this platform has it.
    #[must_use]
    pub fn get<S: ?Sized + 'static>(&self) -> Option<Rc<S>> {
        self.0.get::<S>()
    }

    /// The capability, or an error naming what was missing.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] when nothing is registered for `S`.
    pub fn require<S: ?Sized + 'static>(&self, name: &'static str) -> Result<Rc<S>, ServiceError> {
        self.0.require::<S>(name)
    }

    /// `true` if anything is registered for `S`.
    #[must_use]
    pub fn has<S: ?Sized + 'static>(&self) -> bool {
        self.0.has::<S>()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

// ---------------------------------------------------------------- storage

/// Small values that survive the process.
///
/// Deliberately **not** a database. This is `SharedPreferences` on Android and
/// `NSUserDefaults` on iOS: a settings store for a theme choice, a feature flag,
/// a session token, a "seen the tutorial" bit. An application storing rows wants
/// SQLite, which is a crate away and not this framework's business.
///
/// Strings only, for the same reason. A typed store means a serialisation format
/// baked into the framework, and the application already has one it likes.
///
/// # Why this is synchronous
///
/// Every backing store is a memory-mapped file or an in-process cache, and the
/// calls take microseconds. Making them async so that a hypothetical slow
/// platform could be supported would put a rebuild between a widget and its own
/// settings, forever, on every platform that exists.
pub trait Storage: 'static {
    /// The value for `key`, or `None` if it was never set.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the store could not be read.
    fn get(&self, key: &str) -> Result<Option<String>, ServiceError>;

    /// Store `value` under `key`, replacing anything already there.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the store could not be written.
    fn set(&self, key: &str, value: &str) -> Result<(), ServiceError>;

    /// Forget `key`. Removing something absent is not an error.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the store could not be written.
    fn remove(&self, key: &str) -> Result<(), ServiceError>;

    /// Every key currently stored, in no particular order.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the store could not be read.
    fn keys(&self) -> Result<Vec<String>, ServiceError>;

    /// Forget everything.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the store could not be written.
    fn clear(&self) -> Result<(), ServiceError> {
        for key in self.keys()? {
            self.remove(&key)?;
        }
        Ok(())
    }
}

/// The system pasteboard: what cut, copy and paste move text through.
///
/// Text only, deliberately. A pasteboard can carry images, files and
/// application-defined types, and every one of those needs a representation this
/// crate does not have — so the trait covers what a text field needs and stops
/// there rather than inventing a format nothing can produce. Widening it later
/// adds methods with defaults, which breaks nobody.
///
/// # Why reads return an `Option`
///
/// An empty pasteboard and one holding something that is not text are the same
/// answer to a text field: there is nothing here to paste. Distinguishing them
/// would give a caller a decision it cannot act on.
///
/// # It is not synchronous everywhere, and this trait pretends it is
///
/// On a desktop the pasteboard is a blocking read. On Android and iOS it is a
/// call into the platform that can, in principle, take long enough to matter —
/// and on the web it is asynchronous and permissioned. This signature is the
/// honest one for the two platforms vieww ships to and would have to change for
/// a third. Recorded here rather than discovered later.
pub trait Clipboard: 'static {
    /// The text on the pasteboard, or `None` if there is none.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the pasteboard could not be read, and
    /// [`ServiceError::Denied`] where reading needs a permission that was
    /// refused — which is a real state on some platforms and not the same as
    /// empty.
    fn read_text(&self) -> Result<Option<String>, ServiceError>;

    /// Put `text` on the pasteboard, replacing what was there.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the pasteboard could not be written.
    fn write_text(&self, text: &str) -> Result<(), ServiceError>;
}

/// [`Clipboard`] that only this process can see.
///
/// For tests, and for a platform with no pasteboard of its own. Real rather than
/// a panicking stub for the same reason [`MemoryStorage`] is: copy and paste
/// inside one application work correctly for its whole life, and only sharing
/// with other applications is lost. That is a far better failure than a crash on
/// the first copy.
#[derive(Debug, Default)]
pub struct MemoryClipboard {
    text: std::cell::RefCell<Option<String>>,
}

impl MemoryClipboard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Clipboard for MemoryClipboard {
    fn read_text(&self) -> Result<Option<String>, ServiceError> {
        Ok(self.text.borrow().clone())
    }

    fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        *self.text.borrow_mut() = Some(text.to_string());
        Ok(())
    }
}

/// [`Storage`] that forgets everything when the process ends.
///
/// For tests, and for a platform where nothing better exists. It is a real
/// implementation rather than a panicking stub on purpose: an application
/// running against this behaves correctly for its whole lifetime and only loses
/// data at exit, which is a far better failure than a crash on first write.
#[derive(Debug, Default)]
pub struct MemoryStorage {
    entries: std::cell::RefCell<HashMap<String, String>>,
}

impl MemoryStorage {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Storage for MemoryStorage {
    fn get(&self, key: &str) -> Result<Option<String>, ServiceError> {
        Ok(self.entries.borrow().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), ServiceError> {
        self.entries
            .borrow_mut()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), ServiceError> {
        self.entries.borrow_mut().remove(key);
        Ok(())
    }

    fn keys(&self) -> Result<Vec<String>, ServiceError> {
        Ok(self.entries.borrow().keys().cloned().collect())
    }

    fn clear(&self) -> Result<(), ServiceError> {
        self.entries.borrow_mut().clear();
        Ok(())
    }
}

// --------------------------------------------------------- secure storage

/// Where the keystore is, on a platform that has one.
///
/// Reported by [`SecureStorage::backing`] and worth surfacing rather than
/// hiding, because it is the difference between "an attacker with the file
/// system loses" and "an attacker with the file system wins eventually". An
/// application storing a refresh token can reasonably decline to store one at
/// all on [`Process`](Self::Process).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyBacking {
    /// A secure element or TEE: Android's `StrongBox`- or TEE-backed Keystore,
    /// the Secure Enclave. The key cannot be extracted even from a rooted
    /// device.
    Hardware,
    /// The OS keychain, protected by the user's lock screen but held in
    /// software — Android Keystore without hardware backing, macOS Keychain,
    /// libsecret.
    System,
    /// This process's memory, and nothing else. Correct for a test and a lie
    /// nowhere: [`MemorySecureStorage`] reports it.
    Process,
}

impl KeyBacking {
    /// `true` when the key cannot be extracted from the device at all.
    #[must_use]
    pub const fn is_hardware(self) -> bool {
        matches!(self, Self::Hardware)
    }

    /// `true` when something outside this process is protecting the value.
    ///
    /// The threshold most applications actually care about: it separates "the
    /// OS keychain" from "a `HashMap`", and treating the first two as one is
    /// usually right where treating all three as one never is.
    #[must_use]
    pub const fn survives_the_process(self) -> bool {
        matches!(self, Self::Hardware | Self::System)
    }
}

/// Small values that survive the process **and** are worth stealing.
///
/// The sibling of [`Storage`], deliberately a **separate trait** rather than a
/// flag on it. `docs/AIMS.md` §D asks for an encrypted store as a first-party
/// service; making it a mode of the ordinary one would mean every existing
/// `services.get::<dyn Storage>()` silently keeps working after somebody
/// registers a secure implementation, which is how a session token ends up in
/// `SharedPreferences` beside the theme choice.
///
/// Two traits means the choice is at the call site and visible in review:
/// asking for `dyn SecureStorage` is asking for the keystore, and asking for
/// `dyn Storage` is asking for preferences.
///
/// # Why it is not a supertrait of `Storage` either
///
/// A secure store is not a drop-in for a preferences store. It is slower — every
/// read is a round trip through the OS and may prompt for the user's
/// credentials — and platforms impose hard size limits on it. Code written
/// against [`Storage`] that silently received a keystore would be correct and
/// unusably slow.
///
/// # What it does not promise
///
/// Not that the value is encrypted at rest *by this trait*. The implementation
/// delegates to the platform, and [`backing`](Self::backing) is how it says how
/// well that went. A trait cannot make a secure element exist.
pub trait SecureStorage: 'static {
    /// How well this platform can actually protect what it is given.
    ///
    /// Not a constant: the same binary reports [`KeyBacking::Hardware`] on a
    /// phone with a secure element and [`KeyBacking::System`] on one without, so
    /// an application that cares has to be able to ask at runtime.
    fn backing(&self) -> KeyBacking;

    /// The value for `key`, or `None` if it was never set.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Denied`] when the platform asked for the user's
    /// credentials and did not get them — a real state here with no counterpart
    /// in [`Storage`], and part of why the two signatures are not shared.
    fn get(&self, key: &str) -> Result<Option<String>, ServiceError>;

    /// Store `value` under `key`, replacing anything already there.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the keystore refused the write, and
    /// [`ServiceError::Denied`] if the user did not unlock it.
    fn set(&self, key: &str, value: &str) -> Result<(), ServiceError>;

    /// Forget `key`. Removing something absent is not an error.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the keystore could not be written.
    fn remove(&self, key: &str) -> Result<(), ServiceError>;

    /// Forget everything this application put here.
    ///
    /// Deliberately the only bulk operation, and there is deliberately no
    /// `keys`: a keystore that will enumerate its own contents is one an
    /// attacker enumerates too. An application knows the keys it wrote.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the keystore could not be written.
    fn clear(&self) -> Result<(), ServiceError>;
}

/// [`SecureStorage`] that is secure only in that nothing outside the process
/// can see it.
///
/// For tests, and — reporting [`KeyBacking::Process`], so nobody is misled — for
/// a platform with no keystore at all. Real rather than a panicking stub for
/// [`MemoryStorage`]'s reason, with one difference that carries the weight:
/// because it *says* it is process-backed, an application that declines to store
/// a token without [`KeyBacking::survives_the_process`] makes that decision
/// correctly against this.
#[derive(Debug, Default)]
pub struct MemorySecureStorage {
    entries: std::cell::RefCell<HashMap<String, String>>,
}

impl MemorySecureStorage {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecureStorage for MemorySecureStorage {
    fn backing(&self) -> KeyBacking {
        KeyBacking::Process
    }

    fn get(&self, key: &str) -> Result<Option<String>, ServiceError> {
        Ok(self.entries.borrow().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), ServiceError> {
        self.entries
            .borrow_mut()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), ServiceError> {
        self.entries.borrow_mut().remove(key);
        Ok(())
    }

    fn clear(&self) -> Result<(), ServiceError> {
        self.entries.borrow_mut().clear();
        Ok(())
    }
}

// -------------------------------------------------------------- deep links

/// A link that opened, or reached, the application.
///
/// Kept as the raw string plus the pieces every caller wants, rather than as a
/// parsed URL type: a full URL parser is a dependency this crate does not have
/// and a router only ever needs the path and the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepLink {
    /// Exactly what the platform handed over.
    pub raw: String,
    /// Computed once in [`new`](Self::new).
    ///
    /// Stored rather than derived on demand because a custom scheme's route
    /// needs a leading slash that is not present in `raw` — see
    /// [`path`](Self::path) — so it cannot be a borrow of it.
    path: String,
}

impl DeepLink {
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self::with_authority(raw, Self::scheme_has_authority)
    }

    /// The same, deciding for yourself which schemes carry a host.
    ///
    /// ```
    /// use vieww_foundation::DeepLink;
    ///
    /// // A custom scheme that really does address a host.
    /// let link = DeepLink::with_authority("myapp://tenant.example/orders/17", |scheme| {
    ///     scheme == "myapp" || DeepLink::scheme_has_authority(scheme)
    /// });
    /// assert_eq!(link.path(), "/orders/17");
    ///
    /// // Without saying so, the host is the first path segment — because for
    /// // almost every custom scheme, that is exactly what it is.
    /// assert_eq!(
    ///     DeepLink::new("myapp://tenant.example/orders/17").path(),
    ///     "/tenant.example/orders/17"
    /// );
    /// ```
    ///
    /// # Why this needs asking rather than detecting
    ///
    /// There is nothing in `myapp://a/b` that says whether `a` is a host or the
    /// first segment of the route. Only whoever registered the scheme knows, and
    /// the default — assume not — is right for almost every application scheme
    /// and silently wrong for the few that are not. Getting it wrong eats a path
    /// segment and routes to a screen that is plausible enough to survive
    /// review.
    ///
    /// Compose with [`scheme_has_authority`](Self::scheme_has_authority) rather
    /// than replacing it, or `https` links stop working.
    #[must_use]
    pub fn with_authority(raw: impl Into<String>, has_authority: fn(&str) -> bool) -> Self {
        let raw = raw.into();
        let path = route_of(&raw, has_authority);
        Self { raw, path }
    }

    /// Whether a scheme's `//` is followed by a real host rather than the route.
    ///
    /// The default, and a closed list rather than a guess: every scheme here is
    /// one whose authority is a network host **by specification**. Anything an
    /// application registers for itself is not, and there is no way to tell the
    /// two apart except by knowing — which is what
    /// [`with_authority`](Self::with_authority) is for.
    #[must_use]
    pub fn scheme_has_authority(scheme: &str) -> bool {
        matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "ftps")
    }

    /// The scheme, lowercased — `https`, or a custom one.
    #[must_use]
    pub fn scheme(&self) -> Option<String> {
        let scheme = self.raw.split_once("://")?.0;
        (!scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.'))
        .then(|| scheme.to_ascii_lowercase())
    }

    /// The route, with a leading slash and without the query.
    ///
    /// What a [`Navigator`](https://docs.rs/vieww-widget) route name matches on.
    ///
    /// # The authority is dropped only when the scheme has one
    ///
    /// That is the whole rule, and getting it wrong in either direction is a
    /// bug that ships:
    ///
    /// - `https://example.com/orders/17?ref=email` gives `/orders/17`. The host
    ///   is not part of the route; keeping it gives `/example.com/…`, which
    ///   matches nothing.
    /// - `myapp://orders/17` gives `/orders/17`. A custom scheme has **no
    ///   host** — `orders` is the first segment — and dropping it as an
    ///   authority gives `/17`, which is plausible enough to survive review and
    ///   routes to the wrong screen.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The query parameters, in the order they appeared.
    ///
    /// Percent-decoding is deliberately not done: it needs a table this crate
    /// does not carry, and a caller that needs it has a crate for it. What is
    /// here is the split, which is the part everyone rewrites badly.
    #[must_use]
    pub fn query(&self) -> Vec<(String, String)> {
        let Some((_, query)) = self.raw.split_once('?') else {
            return Vec::new();
        };
        let query = query.split_once('#').map_or(query, |(query, _)| query);
        query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| match pair.split_once('=') {
                Some((key, value)) => (key.to_owned(), value.to_owned()),
                None => (pair.to_owned(), String::new()),
            })
            .collect()
    }
}

/// The route a link addresses. See [`DeepLink::path`].
fn route_of(raw: &str, has_authority: fn(&str) -> bool) -> String {
    let (scheme, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        // No scheme at all — an in-application route written bare.
        None => (String::new(), raw),
    };

    let after_authority = if has_authority(&scheme) {
        match rest.find('/') {
            Some(index) => &rest[index..],
            // A bare host: `https://example.com` addresses the root.
            None => "",
        }
    } else {
        rest
    };

    let path = after_authority
        .split_once(['?', '#'])
        .map_or(after_authority, |(path, _)| path);

    match path {
        "" | "/" => "/".to_owned(),
        path if path.starts_with('/') => path.to_owned(),
        // A custom scheme's first segment, which needs the slash `raw` never
        // had — and the reason this is computed once and stored.
        path => format!("/{path}"),
    }
}

/// Links from outside the application.
///
/// Covers both halves of the platform behaviour, which are genuinely different
/// events and are the classic source of "it works from cold start but not when
/// the app is already open":
///
/// - the link that **launched** the process, available before the first frame;
/// - links delivered to an application that was **already running**.
pub trait DeepLinks: 'static {
    /// The link the application was launched with, if any.
    ///
    /// Taken rather than borrowed: a launch link is delivered once, and a
    /// router that re-reads it on every rebuild would navigate back to it every
    /// time anything else changed.
    fn take_initial(&self) -> Option<DeepLink>;

    /// A link delivered since the last call, if any.
    ///
    /// Polled during a frame rather than delivered by callback, for the reason
    /// [`task`](crate::task) results are: the platform's delivery happens on a
    /// thread and at a time that has nothing to do with the UI, and a signal may
    /// only be written on the UI thread.
    fn take_pending(&self) -> Option<DeepLink>;

    /// Ask the platform to open a link — a web page, a phone number, another
    /// application.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] if the platform cannot, and
    /// [`ServiceError::Failed`] if nothing could handle it.
    fn open(&self, link: &DeepLink) -> Result<(), ServiceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Torch: 'static {
        fn on(&self) -> bool;
    }
    struct Lit;
    impl Torch for Lit {
        fn on(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_service_this_crate_has_never_heard_of_can_be_registered_and_found() {
        // The whole design constraint, as a test. If this ever stops compiling,
        // the seam has stopped being a seam.
        let mut services = Services::new();
        services.provide::<dyn Torch>(Rc::new(Lit));
        assert!(services.get::<dyn Torch>().expect("registered").on());
    }

    #[test]
    fn an_absent_service_is_none_rather_than_a_panic() {
        let services = Services::new();
        assert!(services.get::<dyn Storage>().is_none());
        assert!(!services.has::<dyn Storage>());
    }

    #[test]
    fn requiring_an_absent_service_names_it() {
        let services = Services::new();
        // Destructured rather than `unwrap_err`, which needs the *success* type
        // to be `Debug` — and the success type here is `Rc<dyn Storage>`, a
        // trait object whose implementors this crate has never seen. Requiring
        // `Debug` on every service trait to make one assertion tidier would be
        // the tail wagging the dog.
        let Err(error) = services.require::<dyn Storage>("Storage") else {
            panic!("an absent service must not resolve");
        };
        assert!(error.is_permanent());
        assert!(error.to_string().contains("Storage"), "{error}");
    }

    #[test]
    fn two_services_do_not_collide() {
        let mut services = Services::new();
        services.provide::<dyn Torch>(Rc::new(Lit));
        services.provide::<dyn Storage>(Rc::new(MemoryStorage::new()));
        assert_eq!(services.len(), 2);
        assert!(services.get::<dyn Torch>().is_some());
        assert!(services.get::<dyn Storage>().is_some());
    }

    #[test]
    fn registering_twice_replaces_rather_than_duplicating() {
        let mut services = Services::new();
        services.provide::<dyn Storage>(Rc::new(MemoryStorage::new()));
        services.provide::<dyn Storage>(Rc::new(MemoryStorage::new()));
        assert_eq!(services.len(), 1);
    }

    #[test]
    fn sharing_a_registry_shares_the_service_rather_than_copying_it() {
        let mut services = Services::new();
        services.provide::<dyn Storage>(Rc::new(MemoryStorage::new()));
        let shared = SharedServices::new(services);

        let one = shared.get::<dyn Storage>().expect("registered");
        let two = shared.get::<dyn Storage>().expect("registered");
        one.set("theme", "dark").expect("write");

        assert_eq!(
            two.get("theme").expect("read"),
            Some("dark".to_owned()),
            "two handles must be one store, or two widgets write to two files"
        );
    }

    // ------------------------------------------------------------- storage

    #[test]
    fn memory_storage_round_trips() {
        let storage = MemoryStorage::new();
        assert_eq!(storage.get("k").expect("read"), None);
        storage.set("k", "v").expect("write");
        assert_eq!(storage.get("k").expect("read"), Some("v".to_owned()));
        storage.remove("k").expect("remove");
        assert_eq!(storage.get("k").expect("read"), None);
    }

    #[test]
    fn removing_something_absent_is_not_an_error() {
        assert!(MemoryStorage::new().remove("never-set").is_ok());
    }

    #[test]
    fn clearing_empties_the_store() {
        let storage = MemoryStorage::new();
        storage.set("a", "1").expect("write");
        storage.set("b", "2").expect("write");
        storage.clear().expect("clear");
        assert!(storage.keys().expect("read").is_empty());
    }

    // ----------------------------------------------------------- deep links

    #[test]
    fn a_https_link_splits_into_scheme_path_and_query() {
        let link = DeepLink::new("https://example.com/orders/17?ref=email&x=1");
        assert_eq!(link.scheme().as_deref(), Some("https"));
        assert_eq!(link.path(), "/orders/17");
        assert_eq!(
            link.query(),
            vec![
                ("ref".to_owned(), "email".to_owned()),
                ("x".to_owned(), "1".to_owned())
            ]
        );
    }

    #[test]
    fn a_custom_scheme_has_no_host_so_its_first_segment_is_part_of_the_route() {
        // `orders` is a host by the URL specification and the first segment of
        // the route to every application that has ever used a custom scheme.
        // The specification loses.
        //
        // This test used to assert the opposite of its own name, and the code
        // agreed with the name rather than the assertion — so `myapp://orders/17`
        // routed to `/17`.
        let link = DeepLink::new("myapp://orders/17");
        assert_eq!(link.scheme().as_deref(), Some("myapp"));
        assert_eq!(link.path(), "/orders/17");

        assert_eq!(DeepLink::new("myapp://home").path(), "/home");
    }

    #[test]
    fn an_application_whose_custom_scheme_does_carry_a_host_can_say_so() {
        // **The gap this closes.** The default is right for almost every custom
        // scheme and silently wrong for the few that address a tenant, a server
        // or an account — and being wrong eats a path segment and routes
        // somewhere plausible enough to survive review.
        let multi_tenant: fn(&str) -> bool = |scheme| scheme == "myapp";
        let link = DeepLink::with_authority("myapp://tenant.example/orders/17", multi_tenant);
        assert_eq!(link.path(), "/orders/17");

        // The same link under the default, which is the behaviour that stays.
        assert_eq!(
            DeepLink::new("myapp://tenant.example/orders/17").path(),
            "/tenant.example/orders/17"
        );
    }

    #[test]
    fn an_override_that_forgets_the_web_schemes_breaks_them() {
        // Recorded rather than guarded against: the predicate replaces the
        // default outright, so composing with `scheme_has_authority` is the
        // caller's job and this is what forgetting costs.
        let only_mine: fn(&str) -> bool = |scheme| scheme == "myapp";
        assert_eq!(
            DeepLink::with_authority("https://example.com/orders", only_mine).path(),
            "/example.com/orders",
            "which matches no route — compose with scheme_has_authority"
        );

        let composed: fn(&str) -> bool =
            |scheme| scheme == "myapp" || DeepLink::scheme_has_authority(scheme);
        assert_eq!(
            DeepLink::with_authority("https://example.com/orders", composed).path(),
            "/orders"
        );
    }

    #[test]
    fn a_web_host_is_not_part_of_the_route() {
        // The other direction, which is just as wrong: keeping it would give
        // `/example.com/orders`, and no route matches that.
        assert_eq!(
            DeepLink::new("https://example.com/orders").path(),
            "/orders"
        );
        assert_eq!(DeepLink::new("https://example.com").path(), "/");
        assert_eq!(DeepLink::new("https://example.com/").path(), "/");
    }

    #[test]
    fn a_bare_route_with_no_scheme_is_taken_as_written() {
        assert_eq!(DeepLink::new("/orders/17").path(), "/orders/17");
    }

    #[test]
    fn a_link_with_no_query_has_no_parameters() {
        assert!(DeepLink::new("https://example.com/x").query().is_empty());
    }

    #[test]
    fn a_fragment_does_not_leak_into_the_path_or_the_query() {
        let link = DeepLink::new("https://example.com/x?a=1#section");
        assert_eq!(link.path(), "/x");
        assert_eq!(link.query(), vec![("a".to_owned(), "1".to_owned())]);
    }

    #[test]
    fn a_valueless_parameter_is_kept_with_an_empty_value() {
        let link = DeepLink::new("https://example.com/x?debug");
        assert_eq!(link.query(), vec![("debug".to_owned(), String::new())]);
    }
}
