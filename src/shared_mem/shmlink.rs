use std::{
    alloc::Layout,
    ffi::{CStr, CString},
    fs::File,
    io,
    os::fd::FromRawFd,
};

use anyhow::Result;
use memmap2::MmapMut;

use crate::{
    shared_data::SharedMutexInner,
    shared_mem::{PageAligned, SharedMemorySafe, ShmemWrapper},
};

pub fn shm_open(name: &CStr) -> io::Result<File> {
    let mode = 0o666;
    let options = libc::O_RDWR | libc::O_CREAT;

    match unsafe { libc::shm_open(name.as_ptr(), options, mode) } {
        -1 => Err(io::Error::last_os_error()),
        fd => Ok(unsafe { File::from_raw_fd(fd) }),
    }
}

pub fn shm_unlink(name: &CStr) -> io::Result<()> {
    match unsafe { libc::shm_unlink(name.as_ptr()) } {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

pub fn unlink_if_exists(name: &str) -> io::Result<()> {
    shm_unlink(&into_shm_name(name))
}

fn into_shm_name(path: &str) -> CString {
    let shm_name = format!("/{path}");
    CString::new(shm_name).unwrap()
}

pub struct SharedMem {
    map: MmapMut,
}

impl SharedMem {
    pub unsafe fn new(path: &str, length: usize) -> io::Result<Self> {
        let name = into_shm_name(path);
        let file = shm_open(&name)?;
        file.set_len(u64::try_from(length).unwrap())?;
        let map = unsafe { MmapMut::map_mut(&file) }?;
        Ok(Self { map })
    }

    pub fn as_ptr(&self) -> *mut PageAligned {
        self.map.as_ptr().cast_mut().cast()
    }
}

pub fn get_memory<T: SharedMemorySafe>(name: &str) -> Result<ShmemWrapper> {
    let layout = Layout::new::<SharedMutexInner<T>>();

    let shmem = unsafe { SharedMem::new(name, layout.size()) }
        .map_err(|e| anyhow::anyhow!("Failed to create shared memory: {}", e))?;

    Ok(ShmemWrapper { shmem })
}
