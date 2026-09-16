//! Notifications, local and pushed.

use std::rc::Rc;

use crate::permission::{Grant, Guarded, Permission};
use crate::service::ServiceError;
use crate::task::Task;

/// A group of notifications the user can configure as one.
///
/// Android requires this — a notification posted to no channel does not appear
/// at all on any recent version — and iOS has no equivalent. So it is here
/// because the platform that needs it cannot work without it, and the platform
/// that does not need it can ignore the field. The alternative, an
/// Android-shaped concept smuggled in through a string key, is how a
/// cross-platform API becomes a place people look up workarounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationChannel {
    /// Stable across launches and across updates: the user's own settings for
    /// this group are keyed on it, and changing it silently resets them.
    pub id: String,
    /// Shown in the system settings, so written for the user rather than the
    /// developer: "Delivery updates", not "order_push_v2".
    pub name: String,
    pub description: Option<String>,
    /// Whether it may make a sound and appear as a banner, or should arrive
    /// silently in the shade.
    pub is_prominent: bool,
}

/// When a notification should appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledAt {
    /// As soon as the platform will show it.
    Now,
    /// At a moment, as milliseconds since the Unix epoch.
    ///
    /// Absolute rather than a delay, because a delay is ambiguous across a
    /// suspend: a phone asleep for six hours has not counted them, and "in one
    /// hour" set before bed should still mean an hour.
    Epoch(i64),
}

/// Something to show the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The application's own identifier. Posting twice with one id replaces
    /// rather than duplicates — which is what a progress or a score update
    /// wants, and is the behaviour every platform already has.
    pub id: String,
    pub title: String,
    pub body: String,
    pub channel: Option<String>,
    pub scheduled: ScheduledAt,
    /// Carried through and handed back when the user opens it. The route to
    /// navigate to, usually.
    pub payload: Option<String>,
}

impl Notification {
    /// The simple case: a title and a body, right now.
    #[must_use]
    pub fn now(id: impl Into<String>, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            channel: None,
            scheduled: ScheduledAt::Now,
            payload: None,
        }
    }
}

/// The opaque token a push server addresses this installation by.
///
/// A string, and deliberately not parsed: FCM's and APNs' formats differ, both
/// have changed, and nothing in an application should be reading inside one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushToken(pub String);

/// A registration with the platform's push service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRegistration {
    pub token: PushToken,
    /// `fcm`, `apns`, or whatever a third-party implementation calls itself.
    ///
    /// Present because a server has to know which of its two clients to talk
    /// to, and a token alone does not say.
    pub transport: String,
}

/// Showing notifications, and receiving pushed ones.
///
/// # One trait, two mechanisms, on purpose
///
/// A local notification and a pushed one are different halves of the platform
/// and are often split into two packages. They are together here because from
/// the user's side they are the same thing — a banner — and they share the one
/// permission, the one channel configuration and the one tap handler. Splitting
/// them means two permissions flows for one dialog, which is how an application
/// ends up asking twice.
pub trait Notifications: 'static {
    /// Declare the groups this application posts to.
    ///
    /// Idempotent, and safe to call on every launch: that is how a channel
    /// added in a new version gets created. Unguarded — configuring groups
    /// shows nothing, and Android requires it before permission is even asked.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where the platform has no such concept and
    /// nothing needed doing.
    fn configure(&self, channels: &[NotificationChannel]) -> Result<(), ServiceError>;

    /// Post or schedule one.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the platform rejected it — a schedule in the
    /// past, a channel that was never configured, too many pending.
    fn post(
        &self,
        notification: &Notification,
        grant: &Grant<'_, dyn Notifications>,
    ) -> Result<(), ServiceError>;

    /// Withdraw one that has not fired, or clear one already showing.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where the platform cannot.
    fn cancel(&self, id: &str) -> Result<(), ServiceError>;

    /// Register with the push service and get this installation's token.
    ///
    /// A [`Task`] because it is a network round trip, and one that fails
    /// routinely — on a device with no network, and on an emulator with no
    /// services installed.
    fn register_for_push(
        &self,
        grant: &Grant<'_, dyn Notifications>,
    ) -> Task<PushRegistration, ServiceError>;

    /// The payload of a notification the user tapped, if any, since the last
    /// call.
    ///
    /// Polled during a frame rather than delivered by callback, and taken
    /// rather than borrowed — both for the reasons
    /// [`DeepLinks::take_pending`](crate::service::DeepLinks::take_pending)
    /// gives. A tap that launched the process and a tap on a running one arrive
    /// the same way here, which is the bug this shape exists to prevent.
    fn take_opened(&self) -> Option<String>;

    /// Be told when the push token changes.
    ///
    /// It does change — on reinstall, on restore to a new device, and when the
    /// platform rotates it — and a server addressing the old one is silently
    /// talking to nobody. This is the callback almost every application forgets
    /// to implement, so it is a required method rather than an optional one.
    fn on_token_change(&self, then: Rc<dyn Fn(PushRegistration)>);
}

impl Guarded for dyn Notifications {
    const PERMISSION: Permission = Permission::new("notifications");
}

/// [`Notifications`] in a build that has none.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoNotifications;

impl Notifications for NoNotifications {
    fn configure(&self, _channels: &[NotificationChannel]) -> Result<(), ServiceError> {
        // Nothing to configure and nothing went wrong. An error here would make
        // every launch of a notification-free build log a failure.
        Ok(())
    }

    fn post(
        &self,
        _notification: &Notification,
        _grant: &Grant<'_, dyn Notifications>,
    ) -> Result<(), ServiceError> {
        Err(ServiceError::unsupported("Notifications"))
    }

    fn cancel(&self, _id: &str) -> Result<(), ServiceError> {
        // Cancelling a notification that cannot exist has already achieved what
        // it was asked to, and a caller tidying up on logout should not have to
        // handle a failure for it.
        Ok(())
    }

    fn register_for_push(
        &self,
        _grant: &Grant<'_, dyn Notifications>,
    ) -> Task<PushRegistration, ServiceError> {
        Task::failed(ServiceError::unsupported("Notifications"))
    }

    fn take_opened(&self) -> Option<String> {
        None
    }

    fn on_token_change(&self, _then: Rc<dyn Fn(PushRegistration)>) {}
}
