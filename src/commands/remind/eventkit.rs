//! Provides a high-level wrapper around the eventkit-sys crate defined in this
//! repository. This module allows one to read, create, and update reminders on
//! macOS. Because this module depends on eventkit-sys, do not expect it to
//! compile on non-macOS systems.
use std::{
    ffi::{c_void, CString},
    os::raw::c_char,
    ptr::null_mut,
    slice, str,
    sync::{Arc, Condvar, Mutex},
};

use block::ConcreteBlock;
use chrono::{DateTime, Datelike, TimeZone, Timelike};
use objc::{class, msg_send, runtime::Object, sel, sel_impl};

// NOTE:
//   - calendarItemWithIdentifier to get a reminder
//   - requires a UUID from the reminder
//   - do we have the UUID after we save a reminder?

#[link(name = "EventKit", kind = "framework")]
extern "C" {}

#[derive(Clone, Debug)]
pub(crate) enum EKError {
    /// Used when an operation requires some kind of permissions that the user
    /// has not provided.
    NoAccess,

    /// General case whenever an NSError is encountered. The String is populated
    /// by the NSError's localizedDescription.
    NSError(String),

    /// Used when an operation attempts to retrieve a value that may not be
    /// present.
    NotFound,
}

impl EKError {
    /// The caller of this function must ensure that the *mut Object provided
    /// is, in fact, an NSError nad not some other kind of Object.
    unsafe fn from_ns_error(ns_error: *mut Object) -> EKError {
        let ns_desc = msg_send![ns_error, localizedDescription];
        let desc = from_ns_string(ns_desc);
        EKError::NSError(desc)
    }
}

pub(crate) type EKResult<T> = Result<T, EKError>;

pub(crate) struct EventStore {
    ek_event_store: *mut Object,
}

impl EventStore {
    pub(crate) fn new() -> Self {
        let cls = class!(EKEventStore);
        let mut ek_event_store: *mut Object;
        unsafe {
            ek_event_store = msg_send![cls, alloc];
            ek_event_store = msg_send![ek_event_store, init];
        }

        Self { ek_event_store }
    }

    pub(crate) fn new_with_permission() -> EKResult<Self> {
        let mut event_store = Self::new();
        event_store.request_permission()?;
        Ok(event_store)
    }

    pub(crate) fn request_permission(&mut self) -> EKResult<()> {
        let has_permission = Arc::new(Mutex::new(Ok(false)));
        let has_permission_cond = Arc::new(Condvar::new());
        let completion_block;
        {
            let has_permission = has_permission.clone();
            let has_permission_cond = has_permission_cond.clone();
            completion_block = ConcreteBlock::new(move |granted: bool, ns_error: *mut Object| {
                let mut lock = has_permission.lock().unwrap();
                if ns_error.is_null() {
                    *lock = Ok(granted);
                } else {
                    unsafe {
                        *lock = Err(EKError::from_ns_error(ns_error));
                    }
                }
                has_permission_cond.notify_one();
            })
            .copy();
        }

        let lock = has_permission.lock().unwrap();
        unsafe {
            let _: c_void = msg_send![
                self.ek_event_store,
                requestAccessToEntityType:EKEntityType::Reminder
                completion:completion_block
            ];
        }
        let lock = has_permission_cond.wait(lock).unwrap();

        match &*lock {
            Err(e) => Err(e.clone()),
            Ok(granted) =>
                if *granted {
                    Ok(())
                } else {
                    Err(EKError::NoAccess)
                },
        }
    }

    pub(crate) fn save_reminder(&mut self, reminder: &Reminder, commit: bool) -> EKResult<bool> {
        let mut ns_error: *mut Object = null_mut();
        let saved: bool;
        #[allow(trivial_casts)]
        unsafe {
            saved = msg_send![
                self.ek_event_store,
                saveReminder:reminder.ek_reminder
                commit:commit
                error:&mut (ns_error) as *mut *mut Object
            ];
        }

        if ns_error.is_null() {
            unsafe { return Err(EKError::from_ns_error(ns_error)) }
        }

        Ok(saved)
    }

    pub(crate) fn get_reminder<S: AsRef<str>>(&mut self, uuid: S) -> EKResult<Reminder> {
        let ns_string = to_ns_string(uuid.as_ref().to_string());
        let ek_reminder: *mut Object;
        unsafe {
            ek_reminder = msg_send![self.ek_event_store, calendarItemWithIdentifier: ns_string];
            let _: *mut Object = msg_send![ns_string, release];
        }

        if ek_reminder.is_null() {
            Err(EKError::NotFound)
        } else {
            Ok(Reminder { ek_reminder })
        }
    }
}

impl Drop for EventStore {
    fn drop(&mut self) {
        unsafe {
            let _: c_void = msg_send![self.ek_event_store, release];
        }
    }
}

pub(crate) struct Reminder {
    ek_reminder: *mut Object,
}

impl Reminder {
    pub(crate) fn new(event_store: &mut EventStore) -> Self {
        let cls = class!(EKReminder);
        let ek_reminder: *mut Object;
        unsafe {
            ek_reminder = msg_send![cls, reminderWithEventStore:event_store.ek_event_store];
        }

        let cal: *mut Object;
        unsafe {
            cal = msg_send![event_store.ek_event_store, defaultCalendarForNewReminders];
            let _: c_void = msg_send![ek_reminder, setCalendar: cal];
        }

        Self { ek_reminder }
    }

    pub(crate) fn uuid(&self) -> String {
        let ns_string: *mut Object;
        unsafe {
            ns_string = msg_send![self.ek_reminder, calendarItemIdentifier];
            from_ns_string(ns_string)
        }
    }

    // TODO: this part is probably dangerous + leaks memory. come back here at some
    // point and clean it up.
    pub(crate) fn set_title<S: AsRef<str>>(&mut self, title: S) -> &mut Self {
        let ns_string = to_ns_string(title.as_ref().to_string());
        unsafe {
            let _: c_void = msg_send![self.ek_reminder, setTitle: ns_string];
        }
        self
    }

    pub(crate) fn set_notes<S: AsRef<str>>(&mut self, notes: S) -> &mut Self {
        let ns_string = to_ns_string(notes.as_ref().to_string());
        unsafe {
            let _: c_void = msg_send![self.ek_reminder, setNotes: ns_string];
        }
        self
    }

    pub(crate) fn set_alarm<Tz: TimeZone>(&mut self, date_time: Option<DateTime<Tz>>) -> &mut Self {
        if let Some(date_time) = date_time {
            let ns_date_components = to_ns_date_components(&date_time);
            unsafe {
                let _: c_void =
                    msg_send![self.ek_reminder, setDueDateComponents: ns_date_components];
                let _: c_void = msg_send![ns_date_components, release];
            }
        } else {
            let nil: *mut Object = null_mut();
            unsafe {
                let _: c_void = msg_send![self.ek_reminder, setDueDateComponents: nil];
            }
        }
        self
    }
}

impl Drop for Reminder {
    fn drop(&mut self) {
        unsafe {
            let ns_title: *mut Object = msg_send![self.ek_reminder, title];
            let _: c_void = msg_send![ns_title, release];

            let ns_notes: *mut Object = msg_send![self.ek_reminder, notes];
            let _: c_void = msg_send![ns_notes, release];

            let _: c_void = msg_send![self.ek_reminder, release];
        }
    }
}

/// This is defined in Objective C to be:
///
/// ```
/// enum {
///    EKEntityTypeEvent,
///    EKEntityTypeReminder
/// };
/// typedef NSUInteger EKEntityType;
/// ```
///
/// So we just use a similar enum structure here.
#[repr(u64)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
enum EKEntityType {
    // we don't actually use the Event type ever
    // Event = 0,
    Reminder = 1,
}

/// Converts a str-like to an
/// [NSString](https://developer.apple.com/documentation/foundation/nsstring?language=objc)
/// returning it as a `*mut Object`. It is the responsibility of the caller to
/// free this string.
///
/// # Arguments
///
/// * `s` - The string we want to convert to an NSString. This can be owned or
///   unowned.
fn to_ns_string<S: AsRef<str>>(s: S) -> *mut Object {
    let c_string = CString::new(s.as_ref()).unwrap().into_raw();

    let cls = class!(NSString);
    let mut ns_string: *mut Object;
    unsafe {
        ns_string = msg_send![cls, alloc];
        ns_string = msg_send![ns_string, initWithUTF8String: c_string];
    }

    unsafe {
        let _d = CString::from_raw(c_string);
    }

    ns_string
}

/// Converts an [NSString](https://developer.apple.com/documentation/foundation/nsstring?language=objc)
/// into a [String].
///
/// The provided NSString MUST be UTF8 encoded. This function copies from the
/// NSString, and does not attempt to release it.
unsafe fn from_ns_string(ns_string: *mut Object) -> String {
    #[allow(clippy::ptr_as_ptr)]
    let bytes = {
        let bytes: *const c_char = msg_send![ns_string, UTF8String];
        bytes as *const u8
    };
    let len: usize = msg_send![ns_string, lengthOfBytesUsingEncoding:4]; // 4 = UTF8_ENCODING
    let bytes = slice::from_raw_parts(bytes, len);
    str::from_utf8(bytes).unwrap().to_string()
}

/// Converts a [DateTime] of a particular TZ into its
/// [NSDateComponents](https://developer.apple.com/documentation/foundation/nsdatecomponents?language=objc)
/// counterpart.
///
/// # Arguments
///
/// * `date_time` - The datetime we want to convert.
fn to_ns_date_components<Tz: TimeZone>(date_time: &DateTime<Tz>) -> *mut Object {
    let mut ns_date_components: *mut Object;
    unsafe {
        ns_date_components = msg_send![class!(NSDateComponents), alloc];
        ns_date_components = msg_send![ns_date_components, init];

        let _: c_void = msg_send![ns_date_components, setYear:date_time.year()];
        let _: c_void = msg_send![ns_date_components, setMonth:date_time.month()];
        let _: c_void = msg_send![ns_date_components, setDay:date_time.day()];
        let _: c_void = msg_send![ns_date_components, setHour:date_time.hour()];
        let _: c_void = msg_send![ns_date_components, setMinute:date_time.minute()];
        let _: c_void = msg_send![ns_date_components, setSecond:date_time.second()];
    }
    ns_date_components
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::{Local, NaiveDate};
    use lazy_static::lazy_static;

    use super::*;

    // each test must exclusively own the EventStore
    // so we make sure only one executes at a time.
    lazy_static! {
        static ref MTX: Mutex<()> = Mutex::new(());
    }

    #[test]
    fn test_event_store_new() {
        let _lock = MTX.lock().unwrap();
        let _ = EventStore::new();
    }

    #[test]
    fn test_event_store_new_with_permission() {
        let _lock = MTX.lock().unwrap();
        let _ = EventStore::new_with_permission();
    }

    #[test]
    fn test_to_ns_string() {
        let ns_string: *mut Object;
        unsafe {
            ns_string = to_ns_string("hello world");
            let _: c_void = msg_send![ns_string, release];
        }
    }

    #[test]
    fn test_from_ns_string() {
        let s1 = "hello world".to_string();
        let ns_string = to_ns_string(&s1);
        let s2;
        unsafe {
            s2 = from_ns_string(ns_string);
            let _: c_void = msg_send![ns_string, release];
        }
        assert_eq!(s1, s2);
    }

    #[test]
    fn test_reminder_new() -> EKResult<()> {
        let _lock = MTX.lock().unwrap();
        let mut event_store = EventStore::new()?;
        let _ = Reminder::new(&mut event_store)
            .set_title("a title")
            .set_notes("a notes")
            .set_alarm(Some(Local.from_utc_datetime(
                &NaiveDate::from_ymd(2021, 5, 1).and_hms(12, 0, 0),
            )));
        Ok(())
    }

    #[test]
    fn test_save_reminder() -> EKResult<()> {
        let _lock = MTX.lock().unwrap();
        let mut event_store = EventStore::new()?;
        let mut reminder = Reminder::new(&mut event_store);
        reminder
            .set_title("a title")
            .set_notes("a notes")
            .set_alarm(Some(Local.from_utc_datetime(
                &NaiveDate::from_ymd(2021, 5, 1).and_hms(12, 0, 0),
            )));
        let saved = event_store.save_reminder(&reminder, true)?;
        assert!(saved);
        Ok(())
    }
}
