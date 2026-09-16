//! Permission as part of the seam, rather than part of each service.
//!
//! `docs/AIMS.md` §C names this as the *Safer* aim and as a decision that had to
//! be made **before the second service, not the fifth** — the same reasoning
//! that put [`service`](crate::service) in front of the first one. This module
//! is that decision.
//!
//! # What goes wrong without it
//!
//! Package ecosystems are the worked example. Every plugin that touches a
//! permissioned capability invents its own handling, so asking for the camera
//! looks different in every package: one returns a `bool`, one throws, one has a
//! separate `permission_handler` dependency, and one simply calls the platform
//! and lets the OS kill the process. None of them can stop an application
//! calling the capability without asking — the check and the call are two
//! statements, and forgetting the first is a runtime failure on a user's device.
//!
//! # What is done here instead: a witness token
//!
//! **A guarded capability's methods take a [`Grant`], and a `Grant` cannot be
//! constructed outside this module.** The only thing that hands one out is
//! [`Gate`], and it only does so after reading the permission's actual state. So
//! "call the camera without having resolved the camera permission" is not a
//! thing an application can express — it is not guarded against at runtime, it
//! has no syntax.
//!
//! The whole flow, written as a third party outside this repository would write
//! it, with no vieww change of any kind — which is [`service`](crate::service)'s
//! rule and the reason the mechanism is a token rather than a list:
//!
//! ```
//! use std::rc::Rc;
//! use vieww_foundation::permission::{
//!     AlwaysGranted, Grant, Guarded, Permission, Permissions,
//! };
//! use vieww_foundation::service::{ServiceError, Services};
//!
//! trait Torch: 'static {
//!     fn set(&self, on: bool, grant: &Grant<'_, dyn Torch>) -> Result<(), ServiceError>;
//! }
//!
//! impl Guarded for dyn Torch {
//!     const PERMISSION: Permission = Permission::new("torch");
//! }
//!
//! struct Bulb;
//! impl Torch for Bulb {
//!     fn set(&self, _on: bool, _grant: &Grant<'_, dyn Torch>) -> Result<(), ServiceError> {
//!         Ok(())
//!     }
//! }
//!
//! let mut services = Services::new();
//! services.provide::<dyn Torch>(Rc::new(Bulb));
//! services.provide::<dyn Permissions>(Rc::new(AlwaysGranted));
//!
//! let gate = services.gate::<dyn Torch>().expect("both halves registered");
//! let (torch, grant) = gate.granted().expect("AlwaysGranted grants");
//! torch.set(true, &grant).expect("the bulb is headless");
//! ```
//!
//! The claim that this is *Safer* rather than merely tidy is checked by the
//! compiler, in `compile_fail` doctests rather than assertions — because the
//! thing being asserted is that some code *does not exist*, and no runtime
//! assertion can say that. There are two halves to it.
//!
//! **One: the token cannot be forged.** An application that skipped the
//! permission entirely has nothing to write in the argument position.
//!
//! ```compile_fail
//! use vieww_foundation::permission::{Grant, Guarded, Permission};
//!
//! trait Torch: 'static {
//!     fn set(&self, on: bool, grant: &Grant<'_, dyn Torch>);
//! }
//! impl Guarded for dyn Torch {
//!     const PERMISSION: Permission = Permission::new("torch");
//! }
//!
//! fn light_it(torch: &dyn Torch) {
//!     // There is no way to write this. `Grant` has no public constructor, so
//!     // an application that skipped the permission cannot name the argument.
//!     torch.set(true, &Grant::issue());
//! }
//! ```
//!
//! **Two: the token cannot be kept.** Forging is the obvious attack and the
//! boring one; the failure that actually ships is an application that resolves
//! the permission properly at startup, stashes the proof in a struct, and is
//! still holding it an hour later when the user has revoked the permission from
//! Settings — which Android does to a *running* process. A `Grant` borrows the
//! [`Gate`] that read the state, so outliving that read is a borrow error:
//!
//! ```compile_fail
//! use std::rc::Rc;
//! use vieww_foundation::permission::{AlwaysGranted, Grant, Guarded, Permission, Permissions};
//! use vieww_foundation::service::Services;
//!
//! trait Torch: 'static {
//!     fn set(&self, on: bool, grant: &Grant<'_, dyn Torch>);
//! }
//! impl Guarded for dyn Torch {
//!     const PERMISSION: Permission = Permission::new("torch");
//! }
//! struct Bulb;
//! impl Torch for Bulb {
//!     fn set(&self, _on: bool, _grant: &Grant<'_, dyn Torch>) {}
//! }
//!
//! let mut services = Services::new();
//! services.provide::<dyn Torch>(Rc::new(Bulb));
//! services.provide::<dyn Permissions>(Rc::new(AlwaysGranted));
//!
//! let stashed = {
//!     let gate = services.gate::<dyn Torch>().unwrap();
//!     let (_torch, grant) = gate.granted().unwrap();
//!     // The permission was genuinely resolved here — and the proof of it dies
//!     // with the check, so it cannot be carried to a later frame.
//!     grant
//! };
//! let _ = stashed;
//! ```
//!
//! # Why a token, and what the alternatives cost
//!
//! Four shapes were considered. Naming them here rather than in a commit
//! message because the next person to add a capability will reach for one of the
//! other three, and the reason each was rejected is not obvious from the code
//! that survived.
//!
//! **A runtime check returning `Err(NotPermitted)`.** This is what an ecosystem's
//! ecosystem does and it is the thing §C is a reaction to. It is rejected not
//! because it is unsafe but because it is *unenforced*: the check and the call
//! remain two statements, forgetting the first still compiles, and the failure
//! surfaces on a user's device rather than in CI. It also puts a permission
//! error into every capability's error type, so every call site handles a case
//! that a correctly-ordered program cannot reach.
//!
//! **Typestate on the capability itself** — `Camera<Undetermined>` consumed into
//! `Camera<Granted>`. The cleanest-looking option and the one that does not
//! survive contact with this crate. Capabilities arrive from
//! [`Services`] as `Rc<dyn Camera>`, and a trait object
//! cannot carry a state parameter without giving up the `dyn` lookup that lets a
//! third party register one. Worse, it encodes a falsehood: `Camera<Granted>` is
//! a value that claims permission *for as long as it exists*, and permission is
//! not a property of a value on a platform where the user can revoke it from
//! Settings mid-process. A type that cannot be wrong about the past is being
//! used to make a claim about the future.
//!
//! **A wrapper handed out by the gate** — `GrantedCamera` with the same methods,
//! minus the ceremony. It reads best at the call site and fails the rule
//! [`service`](crate::service) is built on: the wrapper has to re-declare every
//! method of every capability, so a third party's capability needs a wrapper
//! *this crate* would have to write. That is the fork-the-engine failure the
//! seam exists to avoid, and paying it per capability is worse than paying an
//! argument per call.
//!
//! **A token argument**, which is what is here. It composes with `dyn`, it costs
//! nothing at runtime ([`Grant`] is zero-sized, asserted rather than argued), and
//! a third party writes one line — `impl Guarded for dyn Torch` — to join.
//!
//! ## What it costs, stated rather than glossed
//!
//! A design whose common path is unbearable gets routed around, so the price is
//! worth writing down:
//!
//! - **Every guarded method carries an argument that is always `&grant`.** It is
//!   noise, in the signature and at the call site, and it is noise in every
//!   implementation too. This is the real cost and there is no version of the
//!   token design without it.
//! - **Grants do not compose.** A `Grant<'_, dyn Camera>` says nothing about the
//!   microphone, so a helper that records video takes two. That is correct — they
//!   are two permissions the user answers separately — but it is verbose.
//! - **The guard is on the entry point, not on every method.** Deliberately.
//!   [`BarcodeScanner::take_scanned`](crate::mobile::BarcodeScanner::take_scanned)
//!   takes no grant, because draining a queue the application already caused to
//!   fill is not a second use of the camera, and requiring a live grant to read
//!   it would mean re-resolving the permission on every frame. The rule a backend
//!   must follow is therefore: **the method that starts hardware, prompts, or
//!   reaches the network takes a grant; the method that drains what it produced
//!   does not.** Getting this wrong is the one freedom this design leaves, which
//!   is why it is written here rather than assumed.
//!
//! The common path is kept to one call by [`Gate::access`], which asks if it has
//! to, does not if it does not, and hands the closure a grant scoped to itself.
//! An application that only wants to use the thing never names a state.
//!
//! # What this module is deliberately not
//!
//! Not a list of permissions. [`Permission`] is a name, because the set is
//! open: a third party's capability needs a permission this crate has never
//! heard of, and an enum would mean editing this file to add one. The same
//! reasoning as [`Role::Custom`](https://docs.rs/vieww-render) and for the same
//! payoff. It is also what lets one capability sit behind two permissions that
//! the OS genuinely treats as different questions — `location-when-in-use` and
//! `location-always` are two [`Permission`]s, not one with a modifier.
//!
//! Not a model of every *shade* of yes. iOS's limited photo-library access is
//! the hard case: the user granted something, but to four photos rather than the
//! library. It is not a [`PermissionState`] variant, because it is a property of
//! one capability rather than of permission in general, and adding it would make
//! every caller of every other capability match on a variant only the photo
//! library can produce. A capability with shades of yes reports them itself,
//! past the gate, where only its own callers see them.
//!
//! Not a permission *implementation*. Asking the user is the platform layer's
//! job; [`Permissions`] is the shape it plugs into, and [`AlwaysGranted`],
//! [`AlwaysDenied`], [`AlwaysRestricted`] and [`ScriptedPermissions`] are what a
//! headless test uses instead.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use crate::service::{ServiceError, Services, SharedServices};

/// The name of something the user has to agree to.
///
/// A name rather than an enum, because the set is open — see the module docs.
/// Lowercase `snake_case` by convention, matching [`Role::Custom`] and
/// [`SemanticAction::Custom`] in the render crate; the platform layer maps it to
/// whatever the OS calls it (`android.permission.CAMERA`,
/// `NSCameraUsageDescription`).
///
/// [`Role::Custom`]: https://docs.rs/vieww-render
/// [`SemanticAction::Custom`]: https://docs.rs/vieww-render
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Permission(&'static str);

impl Permission {
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Where a permission currently stands.
///
/// # Five states, and each one is a different screen
///
/// The temptation is a `bool`, and the reason to resist it is not tidiness: each
/// variant here has a *different correct response*, and a design that collapses
/// two of them makes the application wrong in both directions at once. Written
/// out, because the collapsing is always done by somebody who believes the two
/// cases are the same:
///
/// | State | What the OS will do if you ask | What the application should show |
/// |---|---|---|
/// | [`Undetermined`](Self::Undetermined) | show the dialog | why you are about to ask |
/// | [`Granted`](Self::Granted) | nothing to ask | the feature |
/// | [`Denied`](Self::Denied) | show the dialog again | the feature, offering to ask |
/// | [`Blocked`](Self::Blocked) | nothing at all | a route to Settings |
/// | [`Restricted`](Self::Restricted) | nothing at all | no feature, and no route |
///
/// The last two rows are the ones that get merged, and merging them produces a
/// "Open Settings" button that leads somewhere the user is not allowed to change
/// anything — which reads as a broken application rather than as a managed
/// device. They are separated here for the same reason
/// [`ServiceError::Unsupported`] is
/// separate from `Failed`: the caller's honest response differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionState {
    /// Never asked. The correct time to explain why you are about to.
    Undetermined,
    /// The user said yes.
    Granted,
    /// The user said no, and asking again would show the dialog.
    ///
    /// Recoverable in place. This is a first refusal on Android, and a refusal
    /// of an in-app pre-prompt anywhere — the state where offering the feature
    /// again is reasonable rather than nagging.
    Denied,
    /// The user said no in a way the OS will not ask about again — "don't ask
    /// again" on Android, any refusal on iOS, where the system dialog is shown
    /// exactly once per install.
    ///
    /// Distinct from [`Denied`](Self::Denied) because the correct response is
    /// different and an application that cannot tell them apart gets it wrong
    /// both ways: it either nags somebody who already refused, or it sends
    /// somebody to Settings who only had to tap "allow".
    ///
    /// The user *can* still fix this, from Settings and not from the
    /// application. See [`needs_settings`](Self::needs_settings).
    Blocked,
    /// Policy forbids it, and the user cannot agree even if they want to.
    ///
    /// Screen Time and parental controls on iOS, a device-owner restriction or a
    /// managed work profile on Android. A real state on both platforms, reported
    /// separately by both, and **not** the same as
    /// [`Blocked`](Self::Blocked) — the user has not refused anything and has no
    /// way to consent. Sending them to Settings shows a switch that is greyed
    /// out, which looks like the application is broken.
    ///
    /// This is a genuinely different product decision: a blocked permission is a
    /// feature the user turned off, and a restricted one is a feature this device
    /// does not have. The honest UI for the second is closer to
    /// [`ServiceError::Unsupported`]
    /// than to a refusal.
    Restricted,
}

impl PermissionState {
    /// `true` only for [`Granted`](Self::Granted).
    #[must_use]
    pub const fn is_granted(self) -> bool {
        matches!(self, Self::Granted)
    }

    /// `true` when asking again could not possibly change the answer.
    ///
    /// Which is every state except the two the OS will still show a dialog for.
    /// [`Gate::access`] branches on this rather than on the variants, so a state
    /// added later does not silently start nagging.
    #[must_use]
    pub const fn is_final(self) -> bool {
        matches!(self, Self::Granted | Self::Blocked | Self::Restricted)
    }

    /// `true` when showing the OS dialog would actually put a question to the
    /// user.
    ///
    /// The exact inverse of [`is_final`](Self::is_final), named from the caller's
    /// side because that is how the decision is phrased at a call site.
    #[must_use]
    pub const fn can_ask(self) -> bool {
        !self.is_final()
    }

    /// `true` when the only remaining remedy is the OS Settings app.
    ///
    /// [`Blocked`](Self::Blocked) alone. Deliberately **not**
    /// [`Restricted`](Self::Restricted), which is the whole reason those are two
    /// variants: offering "Open Settings" for a restriction leads to a control
    /// the user is not permitted to touch.
    #[must_use]
    pub const fn needs_settings(self) -> bool {
        matches!(self, Self::Blocked)
    }
}

impl fmt::Display for PermissionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Undetermined => "undetermined",
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Blocked => "blocked",
            Self::Restricted => "restricted",
        })
    }
}

/// A capability that cannot be used until its permission is resolved.
///
/// Implemented **for the trait object** — `impl Guarded for dyn Camera` — which
/// is what lets [`Gate`] name the permission of a capability it has no other
/// knowledge of. See the module docs for the whole shape.
pub trait Guarded: 'static {
    /// The permission this capability cannot be used without.
    const PERMISSION: Permission;
}

/// Proof that a permission was granted, at the moment it was read.
///
/// Zero-sized, so a guarded method costs exactly what an unguarded one does. Not
/// constructible outside this module, which is the entire point: a capability
/// whose methods take one cannot be called by an application that did not go
/// through [`Gate`].
///
/// # Why it borrows
///
/// The lifetime is the [`Gate`] borrow that produced it, so a grant cannot
/// outlive its own check. Without that, `let grant = gate.granted()` at startup
/// would be a permanent key to a permission the user can revoke from Settings
/// while the process is running — which Android allows and which is exactly the
/// case nobody tests.
///
/// Neither [`Clone`] nor [`Copy`] for the same reason: a copy is a way to widen
/// the lifetime by hand.
pub struct Grant<'a, S: ?Sized> {
    capability: PhantomData<fn() -> *const S>,
    check: PhantomData<&'a ()>,
}

impl<S: ?Sized> fmt::Debug for Grant<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Grant")
    }
}

impl<S: ?Sized> Grant<'_, S> {
    /// The only constructor, and it is private to this module.
    const fn issue() -> Self {
        Self {
            capability: PhantomData,
            check: PhantomData,
        }
    }
}

/// Asks the user, and remembers what they said.
///
/// The platform layer implements this once; every guarded capability in the
/// process shares it. That is the difference from the usual plugin model, where each plugin
/// carries its own copy of this and they disagree.
///
/// # Why requesting takes a callback
///
/// Because the OS dialog is asynchronous on every platform that has one, and a
/// blocking `request` would either be a lie or would block the UI thread that is
/// meant to be drawing the dialog. The callback is invoked on the UI thread,
/// possibly on a later frame, and possibly never — an application backgrounded
/// mid-dialog is a real case, so nothing here promises it will be called.
pub trait Permissions: 'static {
    /// What the OS says right now, without asking the user anything.
    ///
    /// Must not show a dialog. Called during `build`, where showing one would be
    /// a side effect in a function that is required not to have any.
    fn state(&self, permission: Permission) -> PermissionState;

    /// Ask the user, and report what they said.
    ///
    /// `then` runs on the UI thread. Implementations must not call it
    /// synchronously from inside `request` unless the answer needed no dialog —
    /// a caller that rebuilds from the callback would otherwise rebuild from
    /// inside its own build.
    fn request(&self, permission: Permission, then: Rc<dyn Fn(PermissionState)>);
}

/// A capability and the permission standing between it and its caller.
///
/// Obtained from [`Services::gate`] / [`SharedServices::gate`]. Holding one
/// proves the capability exists on this platform and that something is available
/// to resolve permissions; it proves nothing about the user having agreed, which
/// is what [`granted`](Self::granted) is for.
pub struct Gate<S: ?Sized + Guarded> {
    capability: Rc<S>,
    permissions: Rc<dyn Permissions>,
}

impl<S: ?Sized + Guarded> fmt::Debug for Gate<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gate")
            .field("permission", &S::PERMISSION)
            .field("state", &self.state())
            .finish()
    }
}

impl<S: ?Sized + Guarded> Clone for Gate<S> {
    fn clone(&self) -> Self {
        Self {
            capability: Rc::clone(&self.capability),
            permissions: Rc::clone(&self.permissions),
        }
    }
}

impl<S: ?Sized + Guarded> Gate<S> {
    /// Where this capability's permission stands, without asking anything.
    ///
    /// Safe to call during `build`: see [`Permissions::state`].
    #[must_use]
    pub fn state(&self) -> PermissionState {
        self.permissions.state(S::PERMISSION)
    }

    /// The capability and a [`Grant`], if the user has already agreed.
    ///
    /// `None` in every other state, including
    /// [`Blocked`](PermissionState::Blocked) and
    /// [`Restricted`](PermissionState::Restricted) — distinguishing those is
    /// [`state`](Self::state)'s job, and a caller that only wants to use the
    /// thing should not have to enumerate the four ways it could be unavailable.
    /// A caller that wants to *explain* the unavailability asks
    /// [`state`](Self::state) and branches on
    /// [`needs_settings`](PermissionState::needs_settings).
    ///
    /// The grant borrows `self`, so it cannot be kept past this check. That is
    /// the property the whole module exists for.
    #[must_use]
    pub fn granted(&self) -> Option<(&S, Grant<'_, S>)> {
        self.state()
            .is_granted()
            .then(|| (&*self.capability, Grant::issue()))
    }

    /// Ask the user if needed, then run `then` with the capability.
    ///
    /// `then` receives a fresh grant scoped to its own call, so this is the same
    /// guarantee as [`granted`](Self::granted) rather than a way around it. It
    /// does **not** run when the user refuses; the refusal is reported through
    /// `otherwise` so that the ordinary path has no error handling in it.
    ///
    /// Calls `then` synchronously when the permission is already granted, which
    /// is the common case after the first launch — an application should not pay
    /// a frame of latency for a question that was answered last week.
    pub fn access(
        &self,
        then: impl Fn(&S, &Grant<'_, S>) + 'static,
        otherwise: impl Fn(PermissionState) + 'static,
    ) {
        if let Some((capability, grant)) = self.granted() {
            then(capability, &grant);
            return;
        }
        let state = self.state();
        if state.is_final() {
            // Granted was handled above, so this is Blocked or Restricted:
            // asking would show nothing and report the same answer a frame
            // later. The branch is on `is_final` and not on the two variants so
            // that a state added here later cannot silently start nagging —
            // `Restricted` itself arrived after this line was first written.
            //
            // `otherwise` gets the state rather than a bool, because those two
            // want different screens: one offers Settings, the other must not.
            otherwise(state);
            return;
        }
        let capability = Rc::clone(&self.capability);
        self.permissions.request(
            S::PERMISSION,
            Rc::new(move |state: PermissionState| {
                if state.is_granted() {
                    then(&capability, &Grant::issue());
                } else {
                    otherwise(state);
                }
            }),
        );
    }
}

impl Services {
    /// A guarded capability, if this platform has it *and* can resolve
    /// permissions.
    ///
    /// Both halves are needed and neither is assumed: a platform with a camera
    /// and no permission service cannot legally use the camera, and returning a
    /// gate that always answers "undetermined" would be a way to look like it
    /// can.
    #[must_use]
    pub fn gate<S: ?Sized + Guarded>(&self) -> Option<Gate<S>> {
        Some(Gate {
            capability: self.get::<S>()?,
            permissions: self.get::<dyn Permissions>()?,
        })
    }

    /// A guarded capability, or an error naming which half was missing.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] naming `name` when the capability is not
    /// registered, and naming `"Permissions"` when nothing can resolve
    /// permissions — the two are different bugs and the message says which.
    pub fn require_gate<S: ?Sized + Guarded>(
        &self,
        name: &'static str,
    ) -> Result<Gate<S>, ServiceError> {
        Ok(Gate {
            capability: self.require::<S>(name)?,
            permissions: self.require::<dyn Permissions>("Permissions")?,
        })
    }
}

impl SharedServices {
    /// A guarded capability, if this platform has it and can resolve
    /// permissions. See [`Services::gate`].
    #[must_use]
    pub fn gate<S: ?Sized + Guarded>(&self) -> Option<Gate<S>> {
        Some(Gate {
            capability: self.get::<S>()?,
            permissions: self.get::<dyn Permissions>()?,
        })
    }

    /// A guarded capability, or an error naming which half was missing.
    ///
    /// # Errors
    ///
    /// As [`Services::require_gate`].
    pub fn require_gate<S: ?Sized + Guarded>(
        &self,
        name: &'static str,
    ) -> Result<Gate<S>, ServiceError> {
        Ok(Gate {
            capability: self.require::<S>(name)?,
            permissions: self.require::<dyn Permissions>("Permissions")?,
        })
    }
}

/// [`Permissions`] that says yes to everything, without asking.
///
/// For a test, and for a platform where the capability needs no permission at
/// all — a desktop clipboard, a file picker the user just used. Real rather than
/// a panicking stub for the same reason [`MemoryStorage`](crate::MemoryStorage)
/// is.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysGranted;

impl Permissions for AlwaysGranted {
    fn state(&self, _permission: Permission) -> PermissionState {
        PermissionState::Granted
    }

    fn request(&self, _permission: Permission, then: Rc<dyn Fn(PermissionState)>) {
        then(PermissionState::Granted);
    }
}

/// [`Permissions`] that refuses everything, permanently.
///
/// Models the user who tapped "don't ask again" — so
/// [`Blocked`](PermissionState::Blocked) rather than
/// [`Denied`](PermissionState::Denied), because a double that could be asked
/// again is a double a nagging application still passes against.
///
/// For a locked-down work profile or a device under parental controls, reach for
/// [`AlwaysRestricted`] instead: that is a different state and, more to the
/// point, a different screen.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysDenied;

impl Permissions for AlwaysDenied {
    fn state(&self, _permission: Permission) -> PermissionState {
        PermissionState::Blocked
    }

    fn request(&self, _permission: Permission, then: Rc<dyn Fn(PermissionState)>) {
        then(PermissionState::Blocked);
    }
}

/// [`Permissions`] forbidden by policy, everywhere.
///
/// A managed device: Screen Time, parental controls, an MDM profile, an Android
/// device owner. The state an application is least likely to have tried, and one
/// a whole class of users is in on every launch forever — so it is worth a
/// one-liner that runs the entire application in it, rather than a per-permission
/// script somebody has to remember to write.
///
/// The distinction from [`AlwaysDenied`] is the one this double exists for: an
/// application that offers "Open Settings" here sends the user to a switch they
/// are not allowed to touch, and nothing but running in this state finds that.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysRestricted;

impl Permissions for AlwaysRestricted {
    fn state(&self, _permission: Permission) -> PermissionState {
        PermissionState::Restricted
    }

    fn request(&self, _permission: Permission, then: Rc<dyn Fn(PermissionState)>) {
        // Answered without a dialog, because a restriction is not a question.
        // `Gate::access` never reaches here — `is_final` catches it first — but a
        // backend calling `request` directly must get the same answer.
        then(PermissionState::Restricted);
    }
}

/// [`Permissions`] scripted by a test.
///
/// Starts every permission [`Undetermined`](PermissionState::Undetermined) and
/// answers a request with whatever [`will_answer`](Self::will_answer) was told,
/// remembering it afterwards the way a real OS does. Counts requests, so a test
/// can assert the thing that actually goes wrong in applications: asking twice.
///
/// # Answers are a queue, and the last one sticks
///
/// Because the states an application gets wrong are reached by a *sequence*, not
/// by a setting. The one that matters is Android's: the first refusal is
/// [`Denied`](PermissionState::Denied) and re-askable, and the second is
/// [`Blocked`](PermissionState::Blocked) forever. A double that could only be
/// told one answer could not stage that at all, which is how a "denied" path gets
/// tested and a "blocked" path never does.
///
/// ```
/// use std::rc::Rc;
/// use vieww_foundation::permission::{Permission, PermissionState, Permissions, ScriptedPermissions};
///
/// const CAMERA: Permission = Permission::new("camera");
/// let os = ScriptedPermissions::new()
///     .will_answer(CAMERA, PermissionState::Denied)
///     .will_answer(CAMERA, PermissionState::Blocked);
///
/// let noop: Rc<dyn Fn(PermissionState)> = Rc::new(|_| {});
/// os.request(CAMERA, Rc::clone(&noop));
/// assert_eq!(os.state(CAMERA), PermissionState::Denied);
/// os.request(CAMERA, Rc::clone(&noop));
/// assert_eq!(os.state(CAMERA), PermissionState::Blocked);
///
/// // And it stays there. A queue that ran dry and reverted to a default would
/// // be a double that quietly un-blocks a permission the OS never will.
/// os.request(CAMERA, noop);
/// assert_eq!(os.state(CAMERA), PermissionState::Blocked);
/// ```
///
/// The other half of the vocabulary is [`already`](Self::already), which starts a
/// permission in any state — including
/// [`Restricted`](PermissionState::Restricted), which no sequence of asking can
/// ever reach, because policy is decided before the application runs.
#[derive(Debug, Default)]
pub struct ScriptedPermissions {
    answers: RefCell<HashMap<Permission, Vec<PermissionState>>>,
    known: RefCell<HashMap<Permission, PermissionState>>,
    asked: RefCell<Vec<Permission>>,
}

impl ScriptedPermissions {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What the user will say the next time `permission` is requested.
    ///
    /// Called repeatedly to stage a sequence; the final answer repeats for every
    /// request after it. See the type docs for why that is a queue rather than a
    /// setting.
    #[must_use]
    pub fn will_answer(self, permission: Permission, answer: PermissionState) -> Self {
        self.answers
            .borrow_mut()
            .entry(permission)
            .or_default()
            .push(answer);
        self
    }

    /// Start with `permission` already in `state`, as a second launch would.
    ///
    /// The only way to reach [`Restricted`](PermissionState::Restricted), and the
    /// honest one: a managed device is in that state before any of this code
    /// runs.
    #[must_use]
    pub fn already(self, permission: Permission, state: PermissionState) -> Self {
        self.known.borrow_mut().insert(permission, state);
        self
    }

    /// Change `permission` out from under a running application.
    ///
    /// Android lets the user revoke a permission from Settings while the process
    /// is alive, and this is the only way to write a test for what happens next.
    /// Takes `&self` rather than `self` precisely because the interesting moment
    /// is *after* the double has been handed to [`Services`], when the
    /// application is holding a [`Gate`] built on it.
    pub fn set_state(&self, permission: Permission, state: PermissionState) {
        self.known.borrow_mut().insert(permission, state);
    }

    /// How many times the user has been asked about `permission`.
    #[must_use]
    pub fn times_asked(&self, permission: Permission) -> usize {
        self.asked
            .borrow()
            .iter()
            .filter(|&&asked| asked == permission)
            .count()
    }
}

impl Permissions for ScriptedPermissions {
    fn state(&self, permission: Permission) -> PermissionState {
        self.known
            .borrow()
            .get(&permission)
            .copied()
            .unwrap_or(PermissionState::Undetermined)
    }

    fn request(&self, permission: Permission, then: Rc<dyn Fn(PermissionState)>) {
        self.asked.borrow_mut().push(permission);
        let answer = {
            let mut answers = self.answers.borrow_mut();
            match answers.get_mut(&permission) {
                // The last scripted answer is the resting state and is not
                // consumed. Draining the queue and falling back to a default
                // would let a `Blocked` permission become askable again, which
                // no operating system does and which would make the double
                // agree with a nagging application.
                Some(queue) if queue.len() > 1 => queue.remove(0),
                Some(queue) => queue.first().copied().unwrap_or(PermissionState::Denied),
                None => PermissionState::Denied,
            }
        };
        self.known.borrow_mut().insert(permission, answer);
        then(answer);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    const TORCH: Permission = Permission::new("torch");

    /// A third party's capability, written exactly as one outside this repo
    /// would be: its own trait, its own `Guarded` impl, no vieww change.
    trait Torch: 'static {
        fn set(&self, on: bool, grant: &Grant<'_, dyn Torch>) -> bool;
    }

    impl Guarded for dyn Torch {
        const PERMISSION: Permission = TORCH;
    }

    #[derive(Default)]
    struct FakeTorch {
        lit: Cell<bool>,
    }

    impl Torch for FakeTorch {
        fn set(&self, on: bool, _grant: &Grant<'_, dyn Torch>) -> bool {
            self.lit.set(on);
            on
        }
    }

    fn services(permissions: Rc<dyn Permissions>) -> Services {
        let mut services = Services::new();
        services.provide::<dyn Torch>(Rc::new(FakeTorch::default()));
        services.provide::<dyn Permissions>(permissions);
        services
    }

    #[test]
    fn a_capability_with_no_permission_service_has_no_gate() {
        let mut services = Services::new();
        services.provide::<dyn Torch>(Rc::new(FakeTorch::default()));
        assert!(
            services.gate::<dyn Torch>().is_none(),
            "a platform that cannot resolve permissions cannot legally use a \
             guarded capability, and must not be handed a gate that pretends \
             otherwise"
        );
    }

    #[test]
    fn a_permission_service_with_no_capability_has_no_gate() {
        let mut services = Services::new();
        services.provide::<dyn Permissions>(Rc::new(AlwaysGranted));
        assert!(services.gate::<dyn Torch>().is_none());
    }

    #[test]
    fn require_gate_names_which_half_is_missing() {
        let mut services = Services::new();
        services.provide::<dyn Permissions>(Rc::new(AlwaysGranted));
        let error = services.require_gate::<dyn Torch>("Torch").unwrap_err();
        assert_eq!(error, ServiceError::unsupported("Torch"));

        let mut services = Services::new();
        services.provide::<dyn Torch>(Rc::new(FakeTorch::default()));
        let error = services.require_gate::<dyn Torch>("Torch").unwrap_err();
        assert_eq!(error, ServiceError::unsupported("Permissions"));
    }

    #[test]
    fn an_undetermined_permission_hands_out_no_capability() {
        let services = services(Rc::new(ScriptedPermissions::new()));
        let gate = services.gate::<dyn Torch>().unwrap();
        assert_eq!(gate.state(), PermissionState::Undetermined);
        assert!(
            gate.granted().is_none(),
            "never having asked is not the same as having been told yes"
        );
    }

    #[test]
    fn granted_is_the_only_state_that_yields_the_capability() {
        for (state, expected) in [
            (PermissionState::Granted, true),
            (PermissionState::Denied, false),
            (PermissionState::Blocked, false),
            (PermissionState::Restricted, false),
            (PermissionState::Undetermined, false),
        ] {
            let services = services(Rc::new(ScriptedPermissions::new().already(TORCH, state)));
            let gate = services.gate::<dyn Torch>().unwrap();
            assert_eq!(
                gate.granted().is_some(),
                expected,
                "{state} should {} yield the capability",
                if expected { "" } else { "not" }
            );
        }
    }

    #[test]
    fn access_asks_once_and_then_uses_the_remembered_answer() {
        let permissions =
            Rc::new(ScriptedPermissions::new().will_answer(TORCH, PermissionState::Granted));
        let services = services(permissions.clone());
        let gate = services.gate::<dyn Torch>().unwrap();

        let lit = Rc::new(Cell::new(0));
        for _ in 0..3 {
            let lit = Rc::clone(&lit);
            gate.access(
                move |torch, grant| {
                    torch.set(true, grant);
                    lit.set(lit.get() + 1);
                },
                |_| unreachable!("the scripted user agreed"),
            );
        }

        assert_eq!(lit.get(), 3, "the capability ran every time");
        assert_eq!(
            permissions.times_asked(TORCH),
            1,
            "and the user was asked once — nagging is the failure mode this \
             seam exists to make structural rather than remembered"
        );
    }

    #[test]
    fn a_refusal_runs_the_other_arm_and_never_the_capability() {
        let permissions =
            Rc::new(ScriptedPermissions::new().will_answer(TORCH, PermissionState::Denied));
        let services = services(permissions);
        let gate = services.gate::<dyn Torch>().unwrap();

        let refused = Rc::new(Cell::new(None));
        let seen = Rc::clone(&refused);
        gate.access(
            |_, _| unreachable!("the scripted user refused"),
            move |state| seen.set(Some(state)),
        );
        assert_eq!(refused.get(), Some(PermissionState::Denied));
    }

    #[test]
    fn a_blocked_permission_is_not_asked_about_again() {
        let permissions =
            Rc::new(ScriptedPermissions::new().already(TORCH, PermissionState::Blocked));
        let services = services(permissions.clone());
        let gate = services.gate::<dyn Torch>().unwrap();

        gate.access(|_, _| unreachable!(), |_| {});
        assert_eq!(
            permissions.times_asked(TORCH),
            0,
            "the OS would show nothing; asking is a frame of latency and a \
             callback that reports what was already known"
        );
    }

    #[test]
    fn always_denied_blocks_rather_than_merely_refusing() {
        let services = services(Rc::new(AlwaysDenied));
        let gate = services.gate::<dyn Torch>().unwrap();
        assert!(gate.state().is_final());
        assert!(gate.granted().is_none());
    }

    #[test]
    fn a_restriction_is_not_a_refusal_and_offers_no_settings() {
        // The distinction the fifth state exists for. Both are final and both
        // withhold the capability, so a design that stopped at four would look
        // correct here — and would put an "Open Settings" button in front of a
        // user whose device will not let them change it.
        let restricted = services(Rc::new(AlwaysRestricted));
        let gate = restricted.gate::<dyn Torch>().unwrap();
        assert_eq!(gate.state(), PermissionState::Restricted);
        assert!(gate.state().is_final(), "policy will not be asked about");
        assert!(gate.granted().is_none());
        assert!(
            !gate.state().needs_settings(),
            "a restriction has no remedy in Settings, and offering one is the \
             bug that collapsing Blocked and Restricted produces"
        );

        let blocked = services(Rc::new(AlwaysDenied));
        let gate = blocked.gate::<dyn Torch>().unwrap();
        assert!(
            gate.state().needs_settings(),
            "a block, by contrast, is exactly the case Settings fixes"
        );
    }

    #[test]
    fn a_restricted_permission_is_never_asked_about() {
        // A managed device shows no dialog, so asking is a wasted frame and a
        // callback reporting what was already known. The branch that prevents
        // it is `is_final`, which is why adding `Restricted` needed no change
        // to `access`.
        let permissions =
            Rc::new(ScriptedPermissions::new().already(TORCH, PermissionState::Restricted));
        let services = services(permissions.clone());
        let gate = services.gate::<dyn Torch>().unwrap();

        let refused = Rc::new(Cell::new(None));
        let seen = Rc::clone(&refused);
        gate.access(
            |_, _| unreachable!("policy forbids it"),
            move |state| seen.set(Some(state)),
        );
        assert_eq!(refused.get(), Some(PermissionState::Restricted));
        assert_eq!(permissions.times_asked(TORCH), 0);
    }

    #[test]
    fn the_second_refusal_blocks_and_stays_blocked() {
        // Android's real sequence, and the reason the double takes a queue of
        // answers rather than one: an application tested only against `Denied`
        // has never run the path that sends a user to Settings.
        let permissions = Rc::new(
            ScriptedPermissions::new()
                .will_answer(TORCH, PermissionState::Denied)
                .will_answer(TORCH, PermissionState::Blocked),
        );
        let services = services(permissions.clone());
        let gate = services.gate::<dyn Torch>().unwrap();

        let states = Rc::new(RefCell::new(Vec::new()));
        for _ in 0..3 {
            let seen = Rc::clone(&states);
            gate.access(
                |_, _| unreachable!("the scripted user never agreed"),
                move |state| seen.borrow_mut().push(state),
            );
        }

        assert_eq!(
            *states.borrow(),
            vec![
                PermissionState::Denied,
                PermissionState::Blocked,
                PermissionState::Blocked
            ],
            "denied, then blocked, then blocked without another dialog"
        );
        assert_eq!(
            permissions.times_asked(TORCH),
            2,
            "the third attempt read the remembered block instead of asking"
        );

        // And the resting state holds even for a caller that goes around the
        // gate. `Gate::access` short-circuits on `is_final` and so never reaches
        // the queue a third time, which means the stickiness rule is invisible
        // from up here — but a backend calls `request` directly, and a double
        // whose queue ran dry would answer `Denied` and quietly un-block a
        // permission no operating system ever un-blocks.
        let last = Rc::new(Cell::new(None));
        let sink = Rc::clone(&last);
        permissions.request(TORCH, Rc::new(move |state| sink.set(Some(state))));
        assert_eq!(
            last.get(),
            Some(PermissionState::Blocked),
            "the final scripted answer repeats rather than being consumed"
        );
    }

    #[test]
    fn every_state_is_reachable_and_answers_the_three_questions_differently() {
        // The table in `PermissionState`'s docs, as an assertion. Two states
        // agreeing on all three predicates would mean one of them carries no
        // information the other does not, which is the argument for collapsing
        // them — so this is the test that would fail if that were true.
        let all = [
            PermissionState::Undetermined,
            PermissionState::Granted,
            PermissionState::Denied,
            PermissionState::Blocked,
            PermissionState::Restricted,
        ];
        let mut answers: Vec<(bool, bool, bool, String)> = all
            .iter()
            .map(|&state| {
                let staged = services(Rc::new(ScriptedPermissions::new().already(TORCH, state)));
                let gate = staged.gate::<dyn Torch>().unwrap();
                assert_eq!(gate.state(), state, "every state is reachable in a test");
                (
                    state.is_granted(),
                    state.can_ask(),
                    state.needs_settings(),
                    state.to_string(),
                )
            })
            .collect();

        assert!(
            answers
                .iter()
                .all(|&(_, ask, settings, _)| !(ask && settings)),
            "a state that can still be asked about must not also be routed to \
             Settings — that is two remedies offered for one problem"
        );

        let distinct = {
            let mut shapes: Vec<_> = answers
                .iter()
                .map(|&(granted, ask, settings, _)| (granted, ask, settings))
                .collect();
            shapes.sort_unstable();
            shapes.dedup();
            shapes.len()
        };
        assert_eq!(
            distinct, 4,
            "four distinct behaviours across five states: Undetermined and \
             Denied are the one pair that genuinely act alike, and they are kept \
             apart only so the first prompt can be explained rather than sprung"
        );

        answers.sort_by(|a, b| a.3.cmp(&b.3));
        answers.dedup_by(|a, b| a.3 == b.3);
        assert_eq!(answers.len(), 5, "and every one of them prints differently");
    }

    /// The whole point, stated as a test even though the compiler is what
    /// enforces it: there is no path to `Torch::set` that does not go through a
    /// `Grant`, and `Grant` has no public constructor.
    ///
    /// The compile-fail halves — a forged grant and a stashed one — are the two
    /// `compile_fail` doctests in this module's docs, because a test that has to
    /// prove some code *does not compile* cannot be an assertion.
    #[test]
    fn a_grant_is_zero_sized_so_the_guard_costs_nothing() {
        assert_eq!(std::mem::size_of::<Grant<'_, dyn Torch>>(), 0);
    }

    #[test]
    fn a_revoked_permission_stops_yielding_the_capability_immediately() {
        // The case the borrowed lifetime exists for, driven rather than argued.
        // Android revokes from Settings while the process runs; the application
        // is holding the same `Gate` throughout, and the very next check has to
        // notice. It does because a `Gate` reads the state every time rather
        // than caching it — the thing a stashable grant would have defeated.
        let permissions =
            Rc::new(ScriptedPermissions::new().already(TORCH, PermissionState::Granted));
        let services = services(permissions.clone());
        let gate = services.gate::<dyn Torch>().unwrap();

        // The grant lives in a block rather than to the end of the function,
        // which is not a style choice — it is the only way to write this, and it
        // is what the whole design buys. See the `compile_fail` doctest for the
        // version that keeps it.
        {
            let (torch, grant) = gate.granted().expect("granted at startup");
            assert!(torch.set(true, &grant), "and the torch lights");
        }

        permissions.set_state(TORCH, PermissionState::Blocked);
        assert!(
            gate.granted().is_none(),
            "the same gate, one line later, hands out nothing"
        );
        assert!(gate.state().needs_settings());
    }
}
