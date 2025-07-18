use std::{io, sync::atomic::Ordering, time::Duration};

use nix::errno::Errno;

use crate::futex::{
    AosCondition, AosMutex, FUTEX_OWNER_DIED, FUTEX_TID_MASK, duration_to_timespec,
    sys::{cmp_requeue_pi, lock_pi, unlock_pi, wait_requeue_pi},
    tid,
};

pub struct PiCondvar(AosCondition);
impl PiCondvar {
    pub const fn new() -> Self {
        Self(AosCondition::new(0))
    }

    pub fn wait<'a>(&self, guard: PiMutexGuard<'a>) -> io::Result<PiMutexGuard<'a>> {
        self.wait_inner(guard, None)
    }
    pub fn wait_timeout<'a>(
        &self,
        guard: PiMutexGuard<'a>,
        d: Duration,
    ) -> io::Result<PiMutexGuard<'a>> {
        self.wait_inner(guard, Some(d))
    }
    pub fn notify_one(&self, m: &PiMutex) -> io::Result<()> {
        self.wake(m, 0)
    }
    pub fn notify_all(&self, m: &PiMutex) -> io::Result<()> {
        self.wake(m, i32::MAX)
    }

    // ---------- internals ----------
    fn wait_inner<'a>(
        &self,
        guard: PiMutexGuard<'a>,
        dur: Option<Duration>,
    ) -> io::Result<PiMutexGuard<'a>> {
        let start = self.0.load(Ordering::SeqCst);
        // unlock before sleeping
        drop(guard);

        let ts = dur.map(duration_to_timespec);
        unsafe {
            match wait_requeue_pi(&self.0, start, ts, &guard.0.0.futex) {
                Ok(_) => {}
                Err(Errno::ETIMEDOUT) => return Err(io::ErrorKind::TimedOut.into()),
                Err(Errno::EINTR) => return Err(io::ErrorKind::Interrupted.into()),
                Err(e) => return Err(e.into()),
            }
        }

        if guard.0.0.futex.load(Ordering::Acquire) & FUTEX_OWNER_DIED != 0 {
            guard
                .0
                .0
                .futex
                .fetch_and(!FUTEX_OWNER_DIED, Ordering::Relaxed);
        }
        // relock delivered by kernel – create new guard
        Ok(PiMutexGuard(&guard.0))
    }

    fn wake(&self, m: &PiMutex, requeue: i32) -> io::Result<()> {
        let new_gen = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        unsafe { cmp_requeue_pi(&self.0, 1, requeue, &m.0.futex, new_gen) }.map_err(|e| e.into())
    }
}
