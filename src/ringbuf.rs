#[derive(Debug, Clone)]
pub struct RingBuffer<const N: usize> {
    buffer: [f32; N],
    position: usize,
}

impl<const N: usize> Default for RingBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> RingBuffer<N> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: [0.0; N],
            position: 0,
        }
    }

    pub fn push(&mut self, value: f32) {
        self.buffer[self.position] = value;
        self.position = (self.position + 1) % N;
    }

    #[must_use]
    pub fn get(&self, index: usize) -> f32 {
        self.buffer[(self.position + N - 1 - index) % N]
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.position = 0;
    }
}
