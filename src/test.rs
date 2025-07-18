use libc::gettid;

use crate::shared_data::SharedMutex;
#[cfg(not(miri))]
use crate::unlink_if_exists;

use std::{sync::Arc, thread, time::Duration};

macro_rules! function {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f);
        name.strip_suffix("::f").unwrap()
    }};
}

macro_rules! maybe_cleanup {
    () => {
        let _guard = CleanupGuard::new(function!());
    };
}

#[test]
fn test_basic_mutex_operations() {
    maybe_cleanup!();
    let mutex = unsafe { SharedMutex::new_with_val(function!(), 42) };

    {
        let guard = mutex.lock().unwrap();
        assert_eq!(*guard, 42);
    }

    {
        let mut guard = mutex.lock().unwrap();
        *guard = 100;
        assert_eq!(*guard, 100);
    }

    {
        let guard = mutex.try_lock().unwrap().unwrap();
        assert_eq!(*guard, 100);
    }
}

#[test]
fn test_try_lock_fails_when_locked() {
    maybe_cleanup!();
    let mutex = unsafe { SharedMutex::new_with_val(function!(), 0) };

    let _guard = mutex.lock().unwrap();

    assert!(mutex.try_lock().unwrap().is_none());

    drop(_guard);
    let guard = mutex.try_lock().unwrap().unwrap();
    assert_eq!(*guard, 0);
}

#[test]
fn test_multiple_threads_counter() {
    maybe_cleanup!();
    let mutex = Arc::new(unsafe { SharedMutex::new_with_val(function!(), 0) });

    let num_threads = 8;
    let increments_per_thread = 50;

    let handles: Vec<_> = (0..num_threads)
        .map(|_| {
            let mutex = mutex.clone();
            thread::spawn(move || {
                for _ in 0..increments_per_thread {
                    {
                        let mut guard = mutex.lock().unwrap();
                        *guard += 1;
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let final_value = *mutex.lock().unwrap();
    assert_eq!(final_value, num_threads * increments_per_thread);
}

#[test]
fn test_blocking_behavior() {
    maybe_cleanup!();
    let mutex = Arc::new(unsafe { SharedMutex::new_with_val(function!(), "initial") });

    let guard = mutex.lock().unwrap();

    let mutex_clone = mutex.clone();
    let (tx, rx) = std::sync::mpsc::channel();

    let handle = thread::spawn(move || {
        tx.send("thread_started").unwrap();
        let mut guard = mutex_clone.lock().unwrap(); // This should block().unwrap()
        *guard = "modified_by_thread";
        tx.send("thread_acquired_lock().unwrap()").unwrap();
        drop(guard);
        tx.send("thread_released_lock().unwrap()").unwrap();
    });

    assert_eq!(rx.recv().unwrap(), "thread_started");

    thread::sleep(Duration::from_millis(10));

    assert_eq!(*guard, "initial");

    drop(guard);

    assert_eq!(rx.recv().unwrap(), "thread_acquired_lock().unwrap()");
    assert_eq!(rx.recv().unwrap(), "thread_released_lock().unwrap()");

    handle.join().unwrap();

    let final_guard = mutex.lock().unwrap();
    assert_eq!(*final_guard, "modified_by_thread");
}

#[test]
fn test_try_lock_contention() {
    maybe_cleanup!();
    let mutex = Arc::new(unsafe { SharedMutex::new_with_val(function!(), 0) });
    let success_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
    let num_threads = 6;

    let handles: Vec<_> = (0..num_threads)
        .map(|i| {
            let mutex = mutex.clone();
            let success_count = success_count.clone();
            thread::spawn(move || {
                for attempt in 0..20 {
                    if let Some(mut guard) = mutex.try_lock().unwrap() {
                        success_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        *guard += i * 1000 + attempt;
                        thread::sleep(Duration::from_millis(1));
                        drop(guard);
                    }
                    thread::yield_now();
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let total_successes = success_count.load(std::sync::atomic::Ordering::Relaxed);
    assert!(total_successes > 0);
    assert!(total_successes <= num_threads * 20);

    let final_value = *mutex.lock().unwrap();
    assert!(final_value > 0);
}

#[test]
fn test_long_running_operations() {
    maybe_cleanup!();
    let mutex = Arc::new(unsafe { SharedMutex::new_with_val(function!(), 0u64) });
    let num_threads = 4;

    let handles: Vec<_> = (0..num_threads)
        .map(|i| {
            let mutex = mutex.clone();
            thread::spawn(move || {
                for _ in 0..10 {
                    let mut guard = mutex.lock().unwrap();
                    let old_value = *guard;

                    thread::sleep(Duration::from_millis(2));

                    *guard = old_value + (i as u64 + 1);
                    thread::sleep(Duration::from_millis(1));
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let final_value = *mutex.lock().unwrap();
    assert_eq!(final_value, 100);
}

#[test]
fn test_dementia() {
    //! WARNING: dropping the guard and not the mutex will lead to a state where the kernel
    //! can't repair the lock. Do not do this is normal code.
    maybe_cleanup!();
    {
        let mutex = unsafe { SharedMutex::new_with_val(function!(), 0u64) };
        thread::spawn(move || {
            let mut guard = mutex.lock().unwrap();
            *guard = 10;
            std::mem::forget(guard);
            std::mem::forget(mutex);
        })
        .join()
        .unwrap();
    }
    {
        let mutex = unsafe { SharedMutex::new_with_val(function!(), 0u64) };
        thread::spawn(move || {
            let mut guard = mutex.lock().unwrap();
            *guard += 5;
            std::mem::forget(guard);
            std::mem::forget(mutex);
        })
        .join()
        .unwrap()
    }
    let mutex = unsafe { SharedMutex::new_with_val(function!(), 0u64) };
    let final_value = *mutex.lock().unwrap();
    assert_eq!(final_value, 0);
}

#[test]
fn test_panic() {
    maybe_cleanup!();
    {
        let mutex = unsafe { SharedMutex::new_with_val(function!(), 0u64) };
        thread::spawn(move || {
            dbg!(unsafe { gettid() });
            let mut guard = mutex.lock().unwrap();
            *guard = 10;
            panic!()
        })
        .join()
        .unwrap_err();
    }
    {
        let mutex = unsafe { SharedMutex::new_with_val(function!(), 0u64) };
        thread::spawn(move || {
            let mut guard = mutex.lock().unwrap();
            *guard += 5;
            std::mem::forget(guard);
            std::mem::forget(mutex);
        })
        .join()
        .unwrap()
    }
    let mutex = unsafe { SharedMutex::new_with_val(function!(), 0u64) };
    let final_value = *mutex.lock().unwrap();
    assert_eq!(final_value, 0);
}

#[test]
fn test_arc() {
    maybe_cleanup!();
    let mutex = Arc::new(unsafe { SharedMutex::new_with_val(function!(), 0u64) });
    {
        thread::spawn({
            let mutex = mutex.clone();
            move || {
                dbg!(unsafe { gettid() });
                let mut guard = mutex.lock().unwrap();
                *guard = 10;
                std::mem::forget(guard);
            }
        })
        .join()
        .unwrap();
    }
    {
        thread::spawn({
            let mutex = mutex.clone();
            move || {
                let mut guard = mutex.lock().unwrap_err();
                *guard += 5;
                std::mem::forget(guard);
            }
        })
        .join()
        .unwrap()
    }
    let final_value = *mutex.lock().unwrap_err();
    assert_eq!(final_value, 15);
}

#[test]
fn test_panic_poisoning() {
    maybe_cleanup!();
    let mutex = Arc::new(unsafe { SharedMutex::new_with_val(function!(), 42) });

    let mutex_clone = mutex.clone();
    let (tx, rx) = std::sync::mpsc::channel();

    let panic_handle = thread::spawn(move || {
        let mut guard = mutex_clone.lock().unwrap();
        *guard = 999;

        tx.send("lock_acquired").unwrap();

        thread::sleep(Duration::from_millis(10));

        std::mem::forget(guard);
        panic!("Intentional panic while holding lock");
    });

    assert_eq!(rx.recv().unwrap(), "lock_acquired");

    thread::sleep(Duration::from_millis(50));

    assert!(panic_handle.join().is_err());

    match mutex.try_lock() {
        Ok(Some(guard)) => {
            panic!("Lock recovered after panic, value: {}", *guard);
        }
        Ok(None) => {
            panic!("try_lock returned None, but lock should be available (though possibly poisoned)");
        }
        Err(e) => {
            assert_eq!(*e, 999);
            println!("Lock is poisoned as expected: {e:?}");
        }
    }

    let mutex = unsafe { SharedMutex::new_with_val(function!(), 42) };
    let guard = mutex.try_lock().unwrap().unwrap();
    assert_eq!(*guard, 999, "Mutex should've been reset because it had been poisoned");
}

struct CleanupGuard {
    #[allow(dead_code)]
    name: &'static str,
}

impl CleanupGuard {
    fn new(name: &'static str) -> Self {
        #[cfg(not(miri))]
        {
            let _ = unlink_if_exists(name);
        }
        Self { name }
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        #[cfg(not(miri))]
        {
            let _ = unlink_if_exists(self.name);
        }
    }
}
