use std::{io, sync::atomic::Ordering, time::Duration};

use nix::errno::Errno;

use crate::futex::{
    self, AosMutex, FUTEX_OWNER_DIED, FUTEX_TID_MASK, RobustList, duration_to_timespec,
    sys::{lock_pi, unlock_pi},
    tid,
};

pub struct PiMutex(pub(crate) AosMutex);

impl PiMutex {
    pub fn new() -> Self {
        Self(AosMutex::default())
    }

    pub fn lock(&self) -> io::Result<PiMutexGuard<'_>> {
        self.lock_inner(None, true).map(|_| PiMutexGuard(self))
    }
    pub fn lock_timeout(&self, d: Duration) -> io::Result<PiMutexGuard<'_>> {
        self.lock_inner(Some(d), true).map(|_| PiMutexGuard(self))
    }
    pub fn try_lock(&self) -> io::Result<Option<PiMutexGuard<'_>>> {
        match lock_try(&self.0)? {
            true => Ok(Some(PiMutexGuard(self))),
            false => Ok(None),
        }
    }
    pub fn is_locked_by_me(&self) -> bool {
        tid() as u32 & FUTEX_TID_MASK == self.0.futex.load(Ordering::Relaxed)
    }

    pub fn is_locked(&self) -> bool {
        self.0.futex.load(Ordering::Relaxed) != 0
    }

    pub unsafe fn unlock(&self) {
        let next_ptr = &self.0.next as *const _ as *mut RobustList;
        unsafe { futex::robust_remove(next_ptr) };

        let me = tid() as u32;
        if self
            .0
            .futex
            .compare_exchange(me, 0, Ordering::Release, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
        let _ = unsafe { unlock_pi(&self.0.futex) };
    }

    pub(crate) fn lock_inner(&self, dur: Option<Duration>, signals_fail: bool) -> io::Result<()> {
        let me = tid() as u32;
        if self
            .0
            .futex
            .compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            unsafe {
                let next_ptr = &self.0.next as *const _ as *mut RobustList;
                futex::robust_add(next_ptr);
            }
            return Ok(());
        }

        let ts = dur.map(duration_to_timespec);
        loop {
            unsafe {
                match lock_pi(&self.0.futex, ts) {
                    Ok(_) => break,
                    Err(Errno::EINTR) if !signals_fail => continue,
                    Err(Errno::ETIMEDOUT) => {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }

        if self.0.futex.load(Ordering::Acquire) & FUTEX_OWNER_DIED != 0 {
            self.0.futex.fetch_and(!FUTEX_OWNER_DIED, Ordering::Relaxed);
        }

        unsafe {
            let next_ptr = &self.0.next as *const _ as *mut RobustList;
            futex::robust_add(next_ptr);
        }

        Ok(())
    }
}

pub struct PiMutexGuard<'a>(&'a PiMutex);
impl<'a> Drop for PiMutexGuard<'a> {
    fn drop(&mut self) {
        // ignore poisoning on unlock – release is best‑effort
        let _ = unsafe { unlock_pi(&self.0.0.futex) };
    }
}

impl<'a> std::ops::Deref for PiMutexGuard<'a> {
    type Target = PiMutex;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

pub(crate) fn lock_try(m: &AosMutex) -> io::Result<bool> {
    let me = tid() as u32;
    match m
        .futex
        .compare_exchange(0, me, Ordering::AcqRel, Ordering::Relaxed)
    {
        Ok(_) => Ok(true),
        Err(v) if v & FUTEX_OWNER_DIED != 0 => {
            unsafe { lock_pi(&m.futex, None)? };
            Ok(true)
        }
        _ => Ok(false),
    }
}
