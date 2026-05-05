//! ra2m dedicated containers
//!
//! Implement dedicated containers that ease inner mutability with pre-elaboration.
//!

pub mod growonly;
pub mod preallocatedstore;
pub mod prelude;
pub mod sparse_vec;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContainerError {
    #[error("Maximum size exceeded [{0}].")]
    SizeExceeded(usize),
}
