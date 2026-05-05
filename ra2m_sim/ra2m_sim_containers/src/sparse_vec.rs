//! Architecture simulation usually required modeling of large memory.
//! However, those memory are kind of underused during simulation.
//! The aims of SparseVector is to use those "underusage" at their advantage.
//! Instead of allocating all the memory containers upfront, it only allocate virtual space
//! and map the used segment only.
//! Prevent simulation high-ram usage when unecessary

use rustix::mm::{mmap_anonymous, munmap, MapFlags, ProtFlags};
use std::ops::{Deref, DerefMut, Index, IndexMut, Range};
use std::ptr::NonNull;
use std::slice;

pub struct SparseVec<T> {
    ptr: NonNull<T>,
    len: usize,
}

// SAFETY: SparseVec<T> behaves like Vec<T> with respect to thread safety.
//
// Send: SparseVec<T> can be transferred to another thread if T: Send.
//   - SparseVec exclusively owns all T elements in the mmap'd region
//   - No references to the data exist outside of SparseVec
//   - Moving SparseVec transfers ownership of all contained T values
//   - This is identical to Vec<T>'s reasoning for Send
//
// SAFETY: SparseVec<T> behaves like Vec<T> with respect to thread safety.
//
// Sync: &SparseVec<T> can be shared between threads if T: Sync.
//   - A shared reference &SparseVec<T> only provides &T access (via Deref)
//   - If T: Sync, then &T is safe to share across threads
//   - The mmap'd memory itself is just bytes; the synchronization concern
//     is purely about the T values, same as Vec<T>
//
// Note: The raw pointer `NonNull<T>` is !Send and !Sync by default, which
// is why we need explicit unsafe impl. The pointer itself is just an address;
// the safety depends on our ownership semantics, not the pointer.
unsafe impl<T: Send> Send for SparseVec<T> {}
unsafe impl<T: Sync> Sync for SparseVec<T> {}

impl<T: Default + Clone> SparseVec<T> {
    pub fn new(len: usize) -> std::io::Result<Self> {
        if len == 0 {
            return Ok(Self {
                ptr: NonNull::dangling(),
                len: 0,
            });
        }

        let size = len
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| std::io::Error::other("Allocation size overflow"))?;

        // Use rustix crate to have a safe wrapper around mmap syscall
        let raw_ptr = unsafe {
            mmap_anonymous(
                std::ptr::null_mut(), // kernel selected addr
                size,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::PRIVATE | MapFlags::NORESERVE,
            )
        }
        .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;

        let typed_ptr = NonNull::new(raw_ptr.cast::<T>())
            .expect("rustix::mmap returned null pointer despite Ok result");

        // Initialize non-zero-default types
        if !is_zero_valid_default::<T>() {
            for i in 0..len {
                unsafe {
                    typed_ptr.as_ptr().add(i).write(T::default());
                }
            }
        }

        Ok(Self {
            ptr: typed_ptr,
            len,
        })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<T: Default + Clone> SparseVec<T> {
    /// Init begenning of memory from Vector
    /// Take ownership of the underlying data
    pub fn init_from_vec(&mut self, data: Vec<T>) -> std::io::Result<()> {
        if data.len() > self.len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "init data length ({}) exceeds SparseVec length ({})",
                    data.len(),
                    self.len
                ),
            ));
        }

        // SAFETY: Data fits  and both memory regions are owned exclusively
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr.as_ptr(), data.len());
        }

        // Ownership of Vec elements are transfered into SparseVec
        // => Prevent Vec to drop elements moved inside SparseVec
        std::mem::forget(data);

        Ok(())
    }
}

impl<T: Default + Clone> TryFrom<Vec<T>> for SparseVec<T> {
    type Error = std::io::Error;

    fn try_from(value: Vec<T>) -> Result<Self, Self::Error> {
        let mut sparse = Self::new(value.len())?;
        sparse.init_from_vec(value)?;
        Ok(sparse)
    }
}

/// Check if all-zeros is a valid representation of T::default()
fn is_zero_valid_default<T: Default>() -> bool {
    let default = T::default();
    let bytes = unsafe { slice::from_raw_parts(&default as *const T as *const u8, size_of::<T>()) };
    let result = bytes.iter().all(|&b| b == 0);
    std::mem::forget(default);
    result
}

impl<T> Drop for SparseVec<T> {
    fn drop(&mut self) {
        if self.len > 0 {
            unsafe {
                for i in 0..self.len {
                    std::ptr::drop_in_place(self.ptr.as_ptr().add(i));
                }
                let size = self.len * size_of::<T>();
                // munmap can't really fail in practice for valid mappings
                let _ = munmap(self.ptr.as_ptr().cast(), size);
            }
        }
    }
}

impl<T> Deref for SparseVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl<T> DerefMut for SparseVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl<T> Index<usize> for SparseVec<T> {
    type Output = T;
    fn index(&self, idx: usize) -> &T {
        &(**self)[idx]
    }
}

impl<T> IndexMut<usize> for SparseVec<T> {
    fn index_mut(&mut self, idx: usize) -> &mut T {
        &mut (**self)[idx]
    }
}

impl<T> Index<Range<usize>> for SparseVec<T> {
    type Output = [T];
    fn index(&self, range: Range<usize>) -> &[T] {
        &(**self)[range]
    }
}

impl<T> IndexMut<Range<usize>> for SparseVec<T> {
    fn index_mut(&mut self, range: Range<usize>) -> &mut [T] {
        &mut (**self)[range]
    }
}
