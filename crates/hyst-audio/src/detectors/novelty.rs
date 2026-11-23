//! Ported from `src/audio/worklet/brain/novelty.ts`: cosine similarity of a
//! feature vector against a fixed-capacity ring buffer of recent ones. The
//! audio side ticks at a constant hop rate, so a fixed count is a fixed time
//! window.

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom > 1e-6 {
        dot / denom
    } else {
        0.0
    }
}

pub struct NoveltyRingBuffer<const N: usize> {
    buffer: Vec<[f32; N]>,
    write_index: usize,
    capacity: usize,
}

impl<const N: usize> NoveltyRingBuffer<N> {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
            write_index: 0,
            capacity,
        }
    }

    /// Max similarity of `vec` against everything buffered; 0 when empty
    /// ("nothing to be similar to yet" reads as fully novel).
    pub fn max_similarity(&self, vec: &[f32; N]) -> f32 {
        let mut best = 0.0;
        for other in &self.buffer {
            let s = cosine_similarity(vec, other);
            if s > best {
                best = s;
            }
        }
        best
    }

    pub fn push(&mut self, vec: [f32; N]) {
        if self.buffer.len() < self.capacity {
            self.buffer.push(vec);
        } else {
            self.buffer[self.write_index] = vec;
            self.write_index = (self.write_index + 1) % self.capacity;
        }
    }
}
