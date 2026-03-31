//! Clock trait implementations: MonotonicClock, WallClock.
#![allow(unused_variables)]

use std::sync::LazyLock;
use std::time::Duration;

use hyperlight_host::HyperlightError;
use wasi::clocks::{monotonic_clock, wall_clock};

use crate::HostState;
use crate::bindings::wasi;
use crate::wasi_impl::resource::Resource;
use crate::wasi_impl::types::pollable::AnyPollable;

type HlResult<T> = Result<T, HyperlightError>;

static EPOCH: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);

fn now() -> u64 {
    std::time::Instant::now().duration_since(*EPOCH).as_nanos() as u64
}

impl wasi::clocks::MonotonicClock<Resource<AnyPollable>> for HostState {
    fn now(&mut self) -> HlResult<monotonic_clock::Instant> {
        Ok(now())
    }
    fn resolution(&mut self) -> HlResult<monotonic_clock::Duration> {
        Ok(1)
    }
    fn subscribe_instant(
        &mut self,
        when: monotonic_clock::Instant,
    ) -> HlResult<Resource<AnyPollable>> {
        Ok(Resource::new(AnyPollable::future(
            tokio::time::sleep_until(
                tokio::time::Instant::now() + Duration::from_nanos(when.saturating_sub(now())),
            ),
        )))
    }
    fn subscribe_duration(
        &mut self,
        when: monotonic_clock::Duration,
    ) -> HlResult<Resource<AnyPollable>> {
        Ok(Resource::new(AnyPollable::future(tokio::time::sleep(
            Duration::from_nanos(when),
        ))))
    }
}

impl wasi::clocks::WallClock for HostState {
    fn now(&mut self) -> HlResult<wall_clock::Datetime> {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(wall_clock::Datetime {
            seconds: d.as_secs(),
            nanoseconds: d.subsec_nanos(),
        })
    }
    fn resolution(&mut self) -> HlResult<wall_clock::Datetime> {
        Ok(wall_clock::Datetime {
            seconds: 0,
            nanoseconds: 1,
        })
    }
}
