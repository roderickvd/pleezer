//! Fixed-size ring buffer implementation for audio processing.
//!
//! Provides a simple, fixed-size circular buffer optimized for
//! audio applications like noise shaping filters. Stores values
//! in a circular fashion, automatically wrapping around when full.

/// A fixed-size ring buffer for storing floating point values.
///
/// Implements a circular buffer of size N that:
/// * Stores the last N floating point values
/// * Automatically overwrites oldest values when full
/// * Provides zero-based indexing from most recent to oldest
/// * Can be reset to all zeros
///
/// Used primarily for implementing noise shaping filters where
/// previous quantization errors need to be tracked.
#[derive(Debug, Clone)]
pub struct RingBuffer<const N: usize> {
    /// The underlying fixed-size array storing the values
    buffer: [f32; N],

    /// Current write position in the buffer
    position: usize,
}

/// Creates a new empty ring buffer initialized to zeros.
impl<const N: usize> Default for RingBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> RingBuffer<N> {
    /// Creates a new ring buffer of size N, initialized with zeros.
    ///
    /// # Returns
    ///
    /// A new `RingBuffer` instance with all elements set to 0.0
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: [0.0; N],
            position: 0,
        }
    }

    /// Adds a new value to the buffer, overwriting the oldest value if full.
    ///
    /// # Arguments
    ///
    /// * `value` - The new floating point value to add
    pub fn push(&mut self, value: f32) {
        self.buffer[self.position] = value;
        self.position = (self.position + 1) % N;
    }

    /// Retrieves a value from the buffer by index.
    ///
    /// Index 0 returns the most recently added value,
    /// index 1 the second most recent, and so on.
    ///
    /// # Arguments
    ///
    /// * `index` - Zero-based index from most recent to oldest
    ///
    /// # Returns
    ///
    /// The value at the specified index
    #[must_use]
    pub fn get(&self, index: usize) -> f32 {
        self.buffer[(self.position + N - 1 - index) % N]
    }

    /// Resets the buffer to its initial state.
    ///
    /// Sets all values to 0.0 and resets the write position to 0.
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.position = 0;
    }
}
