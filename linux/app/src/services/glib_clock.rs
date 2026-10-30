//! Time and work on the GLib main context (port-plan "Async", shell-design §2.4): fc-core's
//! [`Clock`] (`OffsetDateTime::now_utc()` and `glib::timeout_future`), fc-logic's
//! [`TimeZone`] over `glib::TimeZone::local()`, and the [`Spawn`] every fc-logic model starts its
//! own work with (`glib::MainContext::spawn_local`).
//!
//! Everything here must be used on the thread that owns the default main context (GTK's): the
//! futures are not `Send`, and a timeout attaches to the thread-default context when first polled.

use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::{Clock, LocalBoxFuture};
use fc_logic::calls::Spawn;
use fc_logic::date_culture::TimeZone;
use time::{OffsetDateTime, UtcOffset};

/// Wall time and waiting on GLib (fc-core's `rt::Clock`).
#[derive(Debug, Clone, Copy, Default)]
pub struct GlibClock;

impl Clock for GlibClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    fn sleep(&self, duration: Duration) -> LocalBoxFuture<'static, ()> {
        Box::pin(glib::timeout_future(duration))
    }
}

/// The reader's zone: `glib::TimeZone::local()` (`$TZ`, else `/etc/localtime`), daylight saving
/// included — fc-logic's `TimeZone` (`ChatListModel::with_zone`, carry-forward "Zones").
#[derive(Debug, Clone)]
pub struct GlibZone(glib::TimeZone);

impl GlibZone {
    pub fn local() -> GlibZone {
        GlibZone(glib::TimeZone::local())
    }

    /// A zone by its identifier (`Europe/Belgrade`, `UTC`) — tests.
    #[cfg(test)]
    pub fn named(identifier: &str) -> Option<GlibZone> {
        glib::TimeZone::from_identifier(Some(identifier)).map(GlibZone)
    }
}

impl TimeZone for GlibZone {
    fn offset_at(&self, instant: OffsetDateTime) -> UtcOffset {
        let interval = self
            .0
            .find_interval(glib::TimeType::Universal, instant.unix_timestamp());
        if interval < 0 {
            return UtcOffset::UTC;
        }
        UtcOffset::from_whole_seconds(self.0.offset(interval)).unwrap_or(UtcOffset::UTC)
    }
}

/// fc-logic's `Spawn` on the default main context: the future runs to completion on its own
/// (dropping a GLib `JoinHandle` does not cancel it).
pub fn spawn() -> Spawn {
    Rc::new(|future: LocalBoxFuture<'static, ()>| {
        glib::MainContext::default().spawn_local(future);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use time::macros::datetime;

    #[test]
    fn a_zone_says_its_offset_with_daylight_saving() {
        let belgrade = GlibZone::named("Europe/Belgrade").expect("tzdata in the image");
        assert_eq!(
            belgrade.offset_at(datetime!(2026-01-15 12:00 UTC)),
            UtcOffset::from_hms(1, 0, 0).unwrap()
        );
        assert_eq!(
            belgrade.offset_at(datetime!(2026-07-15 12:00 UTC)),
            UtcOffset::from_hms(2, 0, 0).unwrap()
        );
        let utc = GlibZone::named("UTC").unwrap();
        assert_eq!(
            utc.offset_at(datetime!(2026-07-15 12:00 UTC)),
            UtcOffset::UTC
        );
    }

    #[test]
    fn the_clock_sleeps_on_the_main_context_and_spawn_runs_there() {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let ran = Rc::new(Cell::new(false));
                let flag = Rc::clone(&ran);
                let before = std::time::Instant::now();
                context.block_on(async move {
                    GlibClock.sleep(Duration::from_millis(30)).await;
                    flag.set(true);
                });
                assert!(ran.get());
                assert!(before.elapsed() >= Duration::from_millis(30));
                let now = GlibClock.now();
                assert!((OffsetDateTime::now_utc() - now).abs() < time::Duration::seconds(5));
            })
            .unwrap();
    }
}
