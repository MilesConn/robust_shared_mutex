use std::{
    cell::UnsafeCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use crate::{
    mutex::{PiMutex, lock_try},
    shared_mem::{self, SharedMemorySafe, ShmemWrapper},
};

pub struct SharedMutex<T: SharedMemorySafe> {
    memory: ShmemWrapper,
    _quacks_like_a: PhantomData<Arc<std::sync::Mutex<T>>>,
}

unsafe impl<T: SharedMemorySafe> Send for SharedMutex<T> {}
unsafe impl<T: SharedMemorySafe> Sync for SharedMutex<T> {}

impl<T> SharedMutex<T>
where
    T: SharedMemorySafe,
{
    /// A new mutex with `name` that any process can use. If `name` is not allocated yet
    /// then this function will allocate. In addition, if the mutex is poisoned or unitialized
    /// then `initial` will lazily be used as the init value. If you want to initialize with a
    /// value, then see [`Self::new_with_val`]
    ///
    /// # Safety
    ///
    /// The caller should ensure that for a given name all callers of this function
    /// across any process on the same system, specify the same `T`
    pub unsafe fn new(name: &str, initial: impl FnOnce() -> T) -> SharedMutex<T> {
        let recover_from_poison = true;
        match unsafe { Self::try_new_inner(name, initial, recover_from_poison) } {
            Ok(sm) | Err(sm) => sm,
        }
    }

    unsafe fn try_new_inner(
        name: &str,
        initial: impl FnOnce() -> T,
        recover_from_poison: bool,
    ) -> Result<SharedMutex<T>, SharedMutex<T>> {
        let memory = shared_mem::get_memory::<T>(name).unwrap();

        let shared_mutex: *mut SharedMutexInner<T> = memory.pointer().cast();
        let owner_died = unsafe {
            let owner_died = (*shared_mutex).futex.lock().is_err();
            if (owner_died && recover_from_poison) || !(*shared_mutex).init {
                let data = &raw mut (*shared_mutex).data;
                data.write(UnsafeCell::new(initial()));
                (*shared_mutex).init = true;
            }
            (*shared_mutex).futex.unlock();
            owner_died
        };

        match owner_died {
            false => Ok(SharedMutex {
                memory,
                _quacks_like_a: PhantomData,
            }),
            true => Err(SharedMutex {
                memory,
                _quacks_like_a: PhantomData,
            }),
        }
    }

    /// A new mutex with `name` that any process can use. If `name` is not allocated yet
    /// then this function will allocate. In addition, if the mutex is unitialized
    /// then `initial` will lazily be used as the init value. If the mutex is poisoned
    /// it'll be returned as an error.
    ///
    /// # Safety
    ///
    /// The caller should ensure that for a given name all callers of this function
    /// across any process on the same system, specify the same `T`
    pub unsafe fn try_new(
        name: &str,
        initial: impl FnOnce() -> T,
    ) -> Result<SharedMutex<T>, SharedMutex<T>> {
        let recover_from_poison = false;
        unsafe { Self::try_new_inner(name, initial, recover_from_poison) }
    }

    /// # Safety
    ///
    /// The caller should ensure that for a given name all callers of this function
    /// across any process on the same system, specify the same `T`
    pub unsafe fn new_with_val(name: &str, initial: T) -> SharedMutex<T> {
        unsafe { Self::new(name, || initial) }
    }
}

impl<T: Default + SharedMemorySafe> SharedMutex<T> {
    /// # Safety
    ///
    /// The caller should ensure that for a given name all callers of this function
    /// across any process on the same system, specify the same `T`
    pub unsafe fn from_name(name: &str) -> Self {
        unsafe { Self::new(name, || T::default()) }
    }
}

impl<T> Deref for SharedMutex<T>
where
    T: SharedMemorySafe,
{
    type Target = SharedMutexInner<T>;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.memory.pointer().cast() }
    }
}

#[repr(C)]
pub struct SharedMutexInner<T> {
    futex: PiMutex,
    init: bool,
    data: UnsafeCell<T>,
}

unsafe impl<T: SharedMemorySafe> Send for SharedMutexInner<T> {}
unsafe impl<T: SharedMemorySafe> Sync for SharedMutexInner<T> {}

impl<T: SharedMemorySafe> SharedMutexInner<T> {
    pub fn lock(&self) -> Result<SharedGuard<'_, T>, SharedGuard<'_, T>> {
        match self.futex.lock_inner(None, true) {
            Ok(()) => Ok(SharedGuard {
                data: &self.data,
                futex: &self.futex,
            }),
            Err(_) => Err(SharedGuard {
                data: &self.data,
                futex: &self.futex,
            }),
        }
    }

    /// Locks and ignores if the lock was poisoned or not
    pub fn grab(&self) -> SharedGuard<'_, T> {
        let _ = self.futex.lock();

        SharedGuard {
            data: &self.data,
            futex: &self.futex,
        }
    }

    pub fn try_lock(&self) -> Result<Option<SharedGuard<'_, T>>, SharedGuard<'_, T>> {
        match lock_try(&self.futex.0) {
            Ok(true) => Ok(Some(SharedGuard {
                data: &self.data,
                futex: &self.futex,
            })),
            Ok(false) => Ok(None),
            Err(_) => Err(SharedGuard {
                data: &self.data,
                futex: &self.futex,
            }),
        }
    }

    pub fn is_locked(&self) -> bool {
        self.futex.is_locked()
    }
}

pub struct SharedGuard<'a, T: SharedMemorySafe> {
    data: &'a UnsafeCell<T>,
    futex: &'a PiMutex,
}

impl<'a, T: SharedMemorySafe + std::fmt::Debug> std::fmt::Debug for SharedGuard<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <T as std::fmt::Debug>::fmt(self, f)
    }
}

unsafe impl<'a, T: SharedMemorySafe> Sync for SharedGuard<'a, T> {}

impl<T: SharedMemorySafe> Deref for SharedGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(*self.data).get() }
    }
}

impl<T: SharedMemorySafe> DerefMut for SharedGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *(*self.data).get() }
    }
}

impl<T: SharedMemorySafe> Drop for SharedGuard<'_, T> {
    fn drop(&mut self) {
        unsafe { self.futex.unlock() };
    }
}
