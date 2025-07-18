mod mutex;
pub mod futex;
mod shared_data;
mod robust_list;
mod shared_mem;
#[cfg(test)]
mod test;

pub use shared_data::SharedMutex;
#[cfg(not(miri))]
pub use shared_mem::unlink_if_exists;
