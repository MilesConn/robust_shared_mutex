use std::{
    alloc::Layout,
    collections::HashMap,
    path::Path,
    sync::{Mutex, OnceLock},
};

use anyhow::Result;

use crate::{
    shared_data::SharedMutexInner,
    shared_mem::{PageAligned, SharedMemorySafe, ShmemWrapper},
};

pub(super) fn get_memory<T: SharedMemorySafe>(name: &str) -> Result<ShmemWrapper> {
    #[repr(transparent)]
    struct SendPtr(*mut PageAligned);

    unsafe impl Send for SendPtr {}
    unsafe impl Sync for SendPtr {}

    static TEST_MEMORY: OnceLock<Mutex<HashMap<String, SendPtr>>> = OnceLock::new();

    let memory_map = TEST_MEMORY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = memory_map.lock().unwrap();

    if let Some(ptr) = map.get(&name) {
        return Ok(ShmemWrapper { pointer: ptr.0 });
    }

    let layout = Layout::new::<SharedMutexInner<T>>();
    let raw_ptr = unsafe { std::alloc::alloc_zeroed(layout) as *mut PageAligned };
    map.insert(name.to_string(), SendPtr(raw_ptr));

    Ok(ShmemWrapper { pointer: raw_ptr })
}
