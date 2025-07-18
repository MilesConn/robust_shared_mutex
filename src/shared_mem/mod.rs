use std::alloc::Layout;

use anyhow::Result;

#[cfg(not(miri))]
pub use shmlink::unlink_if_exists;
#[cfg(not(miri))]
use shmlink::SharedMem;

use crate::shared_data::SharedMutexInner;

#[cfg(miri)]
mod mock;
#[cfg(not(miri))]
mod shmlink;

const PAGE_SIZE: usize = 4096;
const _: () = assert!(std::mem::align_of::<PageAligned>() == PAGE_SIZE);

#[repr(align(4096))]
#[expect(dead_code)]
pub struct PageAligned([u8; PAGE_SIZE]);

pub(crate) struct ShmemWrapper {
    #[cfg(not(miri))]
    shmem: SharedMem,
    #[cfg(miri)]
    pointer: *mut PageAligned,
}

impl ShmemWrapper {
    pub(crate) fn pointer(&self) -> *mut PageAligned {
        #[cfg(not(miri))]
        {
            self.shmem.as_ptr()
        }
        #[cfg(miri)]
        {
            self.pointer
        }
    }
}

pub(crate) fn get_memory<T: SharedMemorySafe>(name: &str) -> Result<ShmemWrapper> {
    const {
        let layout = Layout::new::<SharedMutexInner<T>>();
        let page_layout = Layout::new::<PageAligned>();
        assert!(layout.align() <= page_layout.align());
    }
    #[cfg(miri)]
    {
        mock::get_memory::<T>(name)
    }
    #[cfg(not(miri))]
    {
        shmlink::get_memory::<T>(name)
    }
}

pub trait SharedMemorySafe: Copy + Sync {}
impl<T: Copy + Sync> SharedMemorySafe for T {}
