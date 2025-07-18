//! pi_futex.rs – A safe(‑ish) Rust façade for Linux PI‑futexes & robust lists.
//!
//! Public API: [`PiMutex`] and [`PiCondvar`].  Everything else is private
//! glue that stays close to the original C++ implementation.

use std::{cell::OnceCell, io, mem::offset_of, ptr, sync::atomic::AtomicU32, time::Duration};

#[cfg(feature = "tsan")]
use std::mem::MaybeUninit;

use libc::{self, c_int, c_long, pid_t, timespec};
use nix::errno::Errno;

// ---- kernel constants --------------------------------------------------------------------
pub const FUTEX_LOCK_PI: c_int = libc::FUTEX_LOCK_PI;
pub const FUTEX_UNLOCK_PI: c_int = libc::FUTEX_UNLOCK_PI;
pub const FUTEX_WAIT_REQUEUE_PI: c_int = libc::FUTEX_WAIT_REQUEUE_PI;
pub const FUTEX_CMP_REQUEUE_PI: c_int = libc::FUTEX_CMP_REQUEUE_PI;

pub const FUTEX_OWNER_DIED: u32 = libc::FUTEX_OWNER_DIED;
pub const FUTEX_TID_MASK: u32 = libc::FUTEX_TID_MASK;

/// Minimal robust‑list structs (kernel ABI); see linux/futex.h.
#[repr(C)]
pub struct RobustList {
    pub next: *mut RobustList,
}
#[repr(C)]
pub struct RobustListHead {
    pub list: RobustList,
    pub futex_offset: isize,
    pub list_op_pending: *mut RobustList,
}

// ---- C‑layout control blocks --------------------------------------------------------------
#[repr(C)]
pub struct AosMutex {
    pub futex: AtomicU32,
    pub next: usize,
    pub previous: usize,

    #[cfg(feature = "tsan")]
    pub pthread_mutex: libc::pthread_mutex_t,
    #[cfg(feature = "tsan")]
    pub pthread_mutex_init: bool,
}

impl Default for AosMutex {
    fn default() -> Self {
        Self {
            futex: AtomicU32::new(0),
            next: 0,
            previous: 0,
            #[cfg(feature = "tsan")]
            pthread_mutex: unsafe { MaybeUninit::zeroed().assume_init() },
            #[cfg(feature = "tsan")]
            pthread_mutex_init: false,
        }
    }
}

pub type AosCondition = AtomicU32;

thread_local! {
    static MY_TID: std::cell::Cell<pid_t> = const { std::cell::Cell::new(0) };
    static ROBUST: OnceCell<RobustListHead> = const { OnceCell::new() };
}

#[inline]
fn gettid() -> pid_t {
    unsafe { libc::syscall(libc::SYS_gettid) as pid_t }
}

fn ensure_registered(offset: isize) {
    ROBUST.with(|cell| {
        cell.get_or_init(|| {
            let mut head = RobustListHead {
                list: RobustList {
                    next: ptr::null_mut(),
                },
                futex_offset: offset,
                list_op_pending: ptr::null_mut(),
            };
            let sentinel = &mut head.list as *mut _;
            head.list.next = sentinel;

            let r = unsafe {
                libc::syscall(
                    libc::SYS_set_robust_list,
                    &head.list as *const _,
                    std::mem::size_of::<RobustListHead>(),
                )
            };
            debug_assert_eq!(
                r,
                0,
                "set_robust_list failed: {}",
                io::Error::last_os_error()
            );
            head
        });
    });
}

pub fn tid() -> pid_t {
    use std::sync::Once;
    static ONCE: Once = Once::new();

    // fast path
    if let Some(id) = MY_TID.try_with(|t| t.get()).ok().filter(|tid| *tid != 0) {
        return id;
    }

    // slow initialisation
    let id = gettid();
    MY_TID.with(|t| t.set(id));

    let offset = offset_of!(AosMutex, futex) as isize - offset_of!(AosMutex, next) as isize;
    ensure_registered(offset);

    unsafe extern "C" fn atfork_child() {
        MY_TID.with(|t| t.set(0));
    }
    ONCE.call_once(|| unsafe {
        libc::pthread_atfork(None, None, Some(atfork_child));
    });

    id
}

// ---- raw futex syscall --------------------------------------------------------------------
unsafe fn futex_raw(
    uaddr: *const u32,
    op: c_int,
    val: c_int,
    val2: usize,
    uaddr2: *const u32,
    val3: c_int,
) -> nix::Result<c_long> {
    let ret = unsafe { libc::syscall(libc::SYS_futex, uaddr, op, val, val2, uaddr2, val3) };
    if ret == -1 {
        Err(Errno::last())
    } else {
        Ok(ret)
    }
}

// Helpers mirroring old C names (only the ones we need for the safe wrapper)
pub mod sys {
    use super::*;

    #[inline]
    pub unsafe fn lock_pi(addr: &AtomicU32, timeout: Option<timespec>) -> nix::Result<()> {
        unsafe {
            futex_raw(
                addr as *const _ as *const u32,
                FUTEX_LOCK_PI,
                1,
                timeout.map(|t| &t as *const _ as usize).unwrap_or(0),
                ptr::null(),
                0,
            )
        }
        .map(|_| ())
    }
    #[inline]
    pub unsafe fn unlock_pi(addr: &AtomicU32) -> nix::Result<()> {
        unsafe {
            futex_raw(
                addr as *const _ as *const u32,
                FUTEX_UNLOCK_PI,
                0,
                0,
                ptr::null(),
                0,
            )
        }
        .map(|_| ())
    }
    #[inline]
    pub unsafe fn wait_requeue_pi(
        cvar: &AosCondition,
        start: u32,
        timeout: Option<timespec>,
        mtx: &AtomicU32,
    ) -> nix::Result<()> {
        unsafe {
            futex_raw(
                cvar as *const _ as *const u32,
                FUTEX_WAIT_REQUEUE_PI,
                start as _,
                timeout.map(|t| &t as *const _ as usize).unwrap_or(0),
                mtx as *const _ as *const u32,
                0,
            )
        }
        .map(|_| ())
    }
    #[inline]
    pub unsafe fn cmp_requeue_pi(
        cvar: &AosCondition,
        wake: i32,
        requeue: i32,
        mtx: &AtomicU32,
        expected: u32,
    ) -> nix::Result<()> {
        unsafe {
            futex_raw(
                cvar as *const _ as *const u32,
                FUTEX_CMP_REQUEUE_PI,
                wake,
                requeue as usize,
                mtx as *const _ as *const u32,
                expected as _,
            )
        }
        .map(|_| ())
    }
    #[inline]
    pub unsafe fn wait(addr: &AtomicU32, val: u32, timeout: Option<timespec>) -> nix::Result<()> {
        unsafe {
            futex_raw(
                addr as *const _ as *const u32,
                libc::FUTEX_WAIT,
                val as _,
                timeout.map(|t| &t as *const _ as usize).unwrap_or(0),
                ptr::null(),
                0,
            )
        }
        .map(|_| ())
    }
    #[inline]
    pub unsafe fn wake(addr: &AtomicU32, n: i32) -> nix::Result<i32> {
        unsafe {
            futex_raw(
                addr as *const _ as *const u32,
                libc::FUTEX_WAKE,
                n,
                0,
                ptr::null(),
                0,
            )
        }
        .map(|v| v as i32)
    }
}

// ---- tiny helpers reused by safe layer -----------------------------------------------------
#[inline]
pub fn duration_to_timespec(d: Duration) -> timespec {
    timespec {
        tv_sec: d.as_secs() as _,
        tv_nsec: d.subsec_nanos() as _,
    }
}

/// Push `next_ptr` at the front of the current thread's robust list.
///
/// Safety: caller must hold the mutex that owns `next_ptr`.
pub(crate) unsafe fn robust_add(next_ptr: *mut RobustList) {
    // head is guaranteed to be initialised by tid()
    ROBUST.with(|cell| unsafe {
        let head = cell.get().unwrap() as *const _ as *mut RobustListHead;
        (*next_ptr).next = (*head).list.next;
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
        (*head).list.next = next_ptr;
    });
}

/// Unlink `next_ptr` from the thread's robust list (O(N) walk, list is tiny).
///
/// Safety: caller must hold the mutex that owns `next_ptr`.
pub(crate) unsafe fn robust_remove(next_ptr: *mut RobustList) {
    ROBUST.with(|cell| {
        let head = cell.get().unwrap() as *const _ as *mut RobustListHead;
        unsafe {
            let mut prev = &mut (*head).list as *mut RobustList;
            let mut cur = (*prev).next;
            // TODO: review this null check
            while !cur.is_null() && cur != &(*head).list as *const _ as *mut RobustList {
                if cur == next_ptr {
                    (*prev).next = (*cur).next;
                    break;
                }
                prev = cur;
                cur = (*cur).next;
            }
        }
    });
}
