use super::*;

use std::mem::MaybeUninit;
use std::ops::{Index, IndexMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// GrowOnlyVec enable variable number of cores in a Socket
/// Core could only be appended.
/// Use a custom structure instead of Vec to enforce this growable only properties.
/// Enable Thread safety without costly async lock
///
/// A upper-bound is defined on Core vector size due to preallocation of slice
/// This reduce the complexity of lifetime management, no reallocation, thus the
/// given reference have the GrowOnlyVec lifetime
pub struct GrowOnlyVec<T> {
    used: AtomicUsize,
    capacity: usize,
    pool: Box<[MaybeUninit<T>]>,
    inner_mut_guard: Mutex<()>,
}

#[allow(clippy::len_without_is_empty)]
impl<T> GrowOnlyVec<T> {
    pub fn new(capacity: usize) -> Self {
        // Use hand made push instead of macro to prevent trait bound T: Copy
        let mut pool_store = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            pool_store.push(MaybeUninit::uninit());
        }
        let pool = pool_store.into_boxed_slice();
        Self {
            used: AtomicUsize::new(0),
            capacity,
            pool,
            inner_mut_guard: Mutex::new(()),
        }
    }

    pub fn len(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }

    pub fn as_slice(&self) -> &[T] {
        unsafe { self.pool[..self.len()].assume_init_ref() }
    }

    pub fn inner_mut_push(&self, value: T) -> Result<usize, anyhow::Error> {
        // TODO check use of inner_mut_guard
        let _guard = self.inner_mut_guard.lock().unwrap();
        let index = self.used.fetch_add(1_usize, Ordering::SeqCst);
        if index >= self.capacity {
            return Err(ContainerError::SizeExceeded(self.capacity).into());
        } else {
            unsafe {
                let const_ptr = self as *const Self;
                let mut_ptr = const_ptr as *mut Self;
                (*mut_ptr).pool[index] = MaybeUninit::new(value);
            }
        }
        Ok(index)
    }
}

impl<T> Index<usize> for GrowOnlyVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        if index > self.len() {
            panic!(
                "index out of bounds: the len is {} but the index is {}",
                self.len(),
                index
            );
        } else {
            unsafe { self.pool[index].assume_init_ref() }
        }
    }
}

impl<T> IndexMut<usize> for GrowOnlyVec<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        if index > self.len() {
            panic!(
                "index out of bounds: the len is {} but the index is {}",
                self.len(),
                index
            );
        } else {
            unsafe { self.pool[index].assume_init_mut() }
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for GrowOnlyVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_slice(), f)
    }
}

// TODO add dedicated unit-tests
