#![cfg(target_os = "linux")]

/// Kernel ABI for the single “link” node that every mutex contributes.
/// SAFETY: Layout must match <linux/futex.h>.
#[repr(C)]
pub struct RobustList {
    pub next: *mut RobustList,
}

/// Kernel ABI for the per‑thread head –– the kernel expects this exact layout.
#[repr(C)]
pub struct RobustListHead {
    pub list: RobustList,
    /// offset between &mutex->next and &mutex->futex
    pub futex_offset: isize,
    pub list_op_pending: *mut RobustList,
}

impl RobustListHead {
    /// Return the sentinel value `next` should have when the list is empty.
    #[inline]
    fn head_value(&self) -> *mut RobustList {
        &self.list as *const _ as *mut RobustList
    }
}
