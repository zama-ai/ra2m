use serde::{Deserialize, Serialize, Serializer};

/// PreAllocatedStore Provide a fixed amount of slot allocated on init. In case of overflow, it
/// switch to a dynamically allocated space backed by a standard vector. NB: Since protocol will be
/// exclusively wrapped in packet this will draw benefits of allocation already done by Packet.
/// This explain why this custom method is used instead of a simple vec::with_capacity.
///
/// The selected STATIC_STORE should fit with the majority of usecase size. However, to ensure that
/// it work also in corner case, it can be toggled at any time to Dynamic allocation.
#[derive(Deserialize)]
pub enum PreAllocatedStore<T, const STATIC_STORE: usize> {
    #[serde(skip_deserializing)]
    Static(usize, [T; STATIC_STORE]),
    Dynamic(Vec<T>),
}

impl<T, const STATIC_STORE: usize> Default for PreAllocatedStore<T, STATIC_STORE>
where
    T: Default,
{
    fn default() -> Self {
        let static_store = [(); STATIC_STORE].map(|_| Default::default());
        PreAllocatedStore::Static(0, static_store)
    }
}

impl<T, const STATIC_STORE: usize> std::fmt::Debug for PreAllocatedStore<T, STATIC_STORE>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut fmt = f.debug_tuple("PreAllocatedStore");
        // TODO Add a feature flag for data in log and extend the filtering to other structure.
        // match self {
        //     PreAllocatedStore::Static(len, array) => {
        //         fmt.field(&&array[0..*len]);
        //     }
        //     PreAllocatedStore::Dynamic(vec) => {
        //         fmt.field(&vec);
        //     }
        // };
        fmt.finish()
    }
}

#[allow(clippy::len_without_is_empty)]
impl<T, const STATIC_STORE: usize> PreAllocatedStore<T, STATIC_STORE> {
    pub fn len(&self) -> usize {
        match self {
            PreAllocatedStore::Static(len, _) => *len,
            PreAllocatedStore::Dynamic(vec) => vec.len(),
        }
    }

    pub fn as_slice(&self) -> &[T] {
        match self {
            PreAllocatedStore::Static(len, array) => &array[..*len],
            PreAllocatedStore::Dynamic(vec) => vec.as_slice(),
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        match self {
            PreAllocatedStore::Static(len, array) => &mut array[..*len],
            PreAllocatedStore::Dynamic(vec) => vec.as_mut_slice(),
        }
    }
}

impl<T, const STATIC_STORE: usize> PreAllocatedStore<T, STATIC_STORE>
where
    T: Clone,
{
    pub fn push(&mut self, value: T) {
        match self {
            PreAllocatedStore::Static(len, array) => {
                if *len < STATIC_STORE {
                    // use static store
                    array[*len] = value;
                    *len += 1;
                } else {
                    // static store overflow => convert into dynamic
                    let mut vec = array.to_vec();
                    vec.push(value);
                    *self = PreAllocatedStore::Dynamic(vec);
                }
            }
            PreAllocatedStore::Dynamic(vec) => {
                vec.push(value);
            }
        };
    }

    pub fn pop(&mut self) -> Option<T> {
        match self {
            PreAllocatedStore::Static(len, array) => {
                if *len == 0 {
                    None
                } else {
                    *len -= 1;
                    Some(array[*len].clone())
                }
            }
            PreAllocatedStore::Dynamic(vec) => vec.pop(),
        }
    }

    pub fn extend_from_slice(&mut self, other: &[T]) {
        match self {
            PreAllocatedStore::Static(len, array) => {
                if *len + other.len() <= STATIC_STORE {
                    // use static store
                    for v in other.iter() {
                        array[*len] = v.clone();
                        *len += 1;
                    }
                } else {
                    // static store overflow => convert into dynamic
                    let mut vec = array[0..*len].to_vec();
                    vec.extend_from_slice(other);
                    *self = PreAllocatedStore::Dynamic(vec);
                }
            }
            PreAllocatedStore::Dynamic(vec) => {
                vec.extend_from_slice(other);
            }
        };
    }

    pub fn into_vec(self) -> Vec<T> {
        match self {
            PreAllocatedStore::Static(size, array) => array[..size].to_vec(),
            PreAllocatedStore::Dynamic(items) => items,
        }
    }
}

/// Provide a function to easily serialize PreAllocatedStore
/// Handling fixed size array is cumbersome with serde. Instead view PreAllocatedStore as Dynamic
/// variant for Serde purpose
impl<T, const STATIC_STORE: usize> Serialize for PreAllocatedStore<T, STATIC_STORE>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serializer::serialize_newtype_variant(
            serializer,
            "PreAllocatedStore",
            0u32,
            "Dynamic",
            self.as_slice(),
        )
    }
}

#[cfg(test)]
mod units_tests {
    use super::*;

    #[test]
    fn test_preallocatedstore() {
        let mut pas: PreAllocatedStore<usize, 4> = Default::default();
        assert_eq!(pas.as_slice(), []);
        for b in 0..20 {
            pas.push(b);
        }
        assert_eq!(
            *pas.as_slice(),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19]
        );
        println!("PreAllocatedStore after overflow {pas:?}");

        for b in pas.as_mut_slice() {
            *b *= 2;
        }
        assert_eq!(
            *pas.as_slice(),
            [0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38]
        );
        println!("PreAllocatedStore after update {pas:?}");
    }
}
