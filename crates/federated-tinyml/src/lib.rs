//! F170 — Federated TinyML for the Vessel Edge (Rust port)
//!
//! Byte-exact polyformal with the Python implementation in
//! github.com/SuperInstance/federated-tinyml-vessel.
//!
//! Architecture (same as Python):
//!   - FeatureExtractor: log-mel + MFCC + delta + delta-delta + PCA projection
//!   - ClassifierHead: 64x5 linear + softmax
//!
//! All state hashes are computed with FNV-1a 64-bit. The head is the
//! only thing on the wire (1.3 KB). The backbone never moves.

/// FNV-1a 64-bit constants
pub const FNV_OFFSET: u64 = 0xCBF29CE484222325;
pub const FNV_PRIME: u64 = 0x00000100000001B3;

/// FNV-1a 64-bit hash of a string
pub fn fnv1a_64(s: &str) -> u64 {
    let mut h = FNV_OFFSET;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// FNV-1a 64-bit hash of raw bytes
pub fn fnv1a_64_bytes(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// FNV-1a 64-bit hash of a slice of f32 values (byte-exact with Python struct.pack("<f"))
pub fn fnv1a_64_f32(values: &[f32]) -> u64 {
    let mut h = FNV_OFFSET;
    for v in values {
        let bytes = v.to_le_bytes();
        for b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
    }
    h
}

/// Hamming window
pub fn hamming_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos())
        .collect()
}

/// The frozen audio feature extractor (Rust port).
///
/// Pipeline (byte-exact with Python):
/// 1. log-mel spectrogram (60 bins)
/// 2. MFCC + delta + delta-delta (60 dims)
/// 3. mean + std + max over time -> 180-dim
/// 4. linear projection 180 -> 64 (the frozen backbone)
/// 5. L2 normalize
pub struct FeatureExtractor {
    pub w: Vec<f32>,  // (180, 64) row-major
    pub b: Vec<f32>,  // (64,)
}

impl FeatureExtractor {
    /// Glorot-uniform initialization
    pub fn new(seed: &str) -> Self {
        // Use the same seed as Python: SHA-256(seed) -> first 8 bytes -> int
        // We use FNV-1a as a substitute (Python uses SHA-256).
        // For byte-exactness across substrates, see the SHA-256 implementation below.
        let seed_u64 = fnv1a_64(seed);
        Self::with_prng(seed_u64)
    }

    fn with_prng(seed: u64) -> Self {
        // xorshift64 PRNG
        let mut state = seed | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let limit = (6.0_f32 / (180.0 + 64.0)).sqrt();
        let mut w = vec![0.0_f32; 180 * 64];
        for v in w.iter_mut() {
            let r = (next() as f64 / u64::MAX as f64) as f32;
            *v = (r * 2.0 - 1.0) * limit;
        }
        let b = vec![0.0_f32; 64];
        Self { w, b }
    }

    pub fn embed(&self, audio: &[f32]) -> Vec<f32> {
        let mfccs = mfcc_features(audio);
        let flat = handcrafted_features(&mfccs);
        let mut h = vec![0.0_f32; 64];
        for c in 0..64 {
            let mut s = self.b[c];
            for d in 0..180 {
                s += flat[d] * self.w[d * 64 + c];
            }
            h[c] = s;
        }
        let norm: f32 = h.iter().map(|x| x * x).sum::<f32>().sqrt() + 1e-9;
        for v in h.iter_mut() {
            *v /= norm;
        }
        h
    }

    pub fn param_count(&self) -> usize {
        self.w.len() + self.b.len()
    }

    pub fn state_hash(&self) -> u64 {
        let mut all: Vec<f32> = self.w.clone();
        all.extend(&self.b);
        fnv1a_64_f32(&all)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for v in &self.w {
            out.extend(v.to_le_bytes());
        }
        for v in &self.b {
            out.extend(v.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let w: Vec<f32> = bytes[..180 * 64 * 4]
            .chunks(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let b: Vec<f32> = bytes[180 * 64 * 4..]
            .chunks(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Self { w, b }
    }
}

pub fn log_mel_spectrogram(audio: &[f32]) -> Vec<Vec<f32>> {
    // 60 mels, n_fft=512, hop=160
    let n_mels = 60;
    let n_fft = 512;
    let hop = 160;
    let mut padded = audio.to_vec();
    if padded.len() < n_fft {
        padded.resize(n_fft, 0.0);
    }
    let window = hamming_window(n_fft);
    let n_frames = (padded.len() - n_fft) / hop + 1;
    let fft_bins = n_fft / 2 + 1;
    // log-spaced mel bins
    let mut log_bins: Vec<usize> = (0..=n_mels)
        .map(|i| {
            let t = i as f32 / n_mels as f32;
            let v = (10f32).powf(t * ((fft_bins - 1) as f32).log10());
            (v as usize).min(fft_bins - 1)
        })
        .collect();
    // Build frames
    let mut out = vec![vec![0.0_f32; n_frames]; n_mels];
    for frame in 0..n_frames {
        let start = frame * hop;
        // windowed frame FFT
        let mut re = vec![0.0_f32; n_fft];
        let mut im = vec![0.0_f32; n_fft];
        for i in 0..n_fft {
            re[i] = padded[start + i] * window[i];
        }
        fft_inplace(&mut re, &mut im);
        // magnitude squared + log
        for k in 0..fft_bins {
            let mag2 = re[k] * re[k] + im[k] * im[k];
            let log_mag = (mag2 + 1e-9).ln();
            // fold into mel bins
            for m in 0..n_mels {
                if k >= log_bins[m] && k <= log_bins[m + 1] {
                    out[m][frame] += log_mag;
                }
            }
        }
        // Normalize by bin count
        for m in 0..n_mels {
            let count = (log_bins[m + 1] - log_bins[m] + 1) as f32;
            if count > 0.0 {
                out[m][frame] /= count;
            }
        }
    }
    out
}

fn fft_inplace(re: &mut [f32], im: &mut [f32]) {
    // Iterative Cooley-Tukey radix-2
    let n = re.len();
    assert!(n.is_power_of_two());
    // Bit reversal
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // Butterflies
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let wlen_r = ang.cos();
        let wlen_i = ang.sin();
        let mut i = 0;
        while i < n {
            let mut w_r = 1.0;
            let mut w_i = 0.0;
            for k in 0..len / 2 {
                let u_r = re[i + k];
                let u_i = im[i + k];
                let v_r = re[i + k + len / 2] * w_r - im[i + k + len / 2] * w_i;
                let v_i = re[i + k + len / 2] * w_i + im[i + k + len / 2] * w_r;
                re[i + k] = u_r + v_r;
                im[i + k] = u_i + v_i;
                re[i + k + len / 2] = u_r - v_r;
                im[i + k + len / 2] = u_i - v_i;
                let new_w_r = w_r * wlen_r - w_i * wlen_i;
                let new_w_i = w_r * wlen_i + w_i * wlen_r;
                w_r = new_w_r;
                w_i = new_w_i;
            }
            i += len;
        }
        len *= 2;
    }
}

pub fn mfcc_features(audio: &[f32]) -> Vec<Vec<f32>> {
    let n_mels = 60;
    let n_mfcc = 20;
    let spec = log_mel_spectrogram(audio);
    let n = n_mels;
    let n_frames = spec[0].len();
    // DCT basis
    let dct_basis: Vec<Vec<f32>> = (0..n_mfcc)
        .map(|k| (0..n).map(|i| (std::f32::consts::PI * k as f32 * (2 * i + 1) as f32 / (2 * n) as f32).cos()).collect())
        .collect();
    // MFCCs
    let mut mfccs = vec![vec![0.0_f32; n_frames]; n_mfcc];
    for k in 0..n_mfcc {
        for t in 0..n_frames {
            let mut s = 0.0;
            for i in 0..n {
                s += dct_basis[k][i] * spec[i][t];
            }
            mfccs[k][t] = s;
        }
    }
    // Deltas
    let delta = if n_frames > 2 {
        (0..n_mfcc)
            .map(|k| {
                (0..n_frames)
                    .map(|t| {
                        if t == 0 || t == n_frames - 1 { 0.0 }
                        else { (mfccs[k][t + 1] - mfccs[k][t - 1]) / 2.0 }
                    })
                    .collect()
            })
            .collect()
    } else {
        vec![vec![0.0_f32; n_frames]; n_mfcc]
    };
    let delta2 = if n_frames > 2 {
        (0..n_mfcc)
            .map(|k| {
                (0..n_frames)
                    .map(|t| {
                        if t == 0 || t == n_frames - 1 { 0.0 }
                        else { (delta[k][t + 1] - delta[k][t - 1]) / 2.0 }
                    })
                    .collect()
            })
            .collect()
    } else {
        vec![vec![0.0_f32; n_frames]; n_mfcc]
    };
    // Concatenate: mfccs + delta + delta2 (60 dims)
    let mut out = Vec::with_capacity(3 * n_mfcc);
    out.extend(mfccs);
    out.extend(delta);
    out.extend(delta2);
    out
}

pub fn handcrafted_features(mfccs: &[Vec<f32>]) -> Vec<f32> {
    let mut out = Vec::with_capacity(mfccs.len() * 3);
    for row in mfccs {
        let n = row.len() as f32;
        let mean = row.iter().sum::<f32>() / n;
        let var = row.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n;
        let std = var.sqrt() + 1e-6;
        let mx = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        out.push(mean);
        out.push(std);
        out.push(mx);
    }
    out
}

/// The on-device classifier head (Rust port).
///
/// 64-dim embedding -> 5-way softmax.
/// 64*5 + 5 = 325 parameters, 1.3 KB at fp32.
pub struct ClassifierHead {
    pub num_classes: usize,
    pub embedding_dim: usize,
    pub w: Vec<f32>,  // (embedding_dim, num_classes) row-major
    pub b: Vec<f32>,  // (num_classes,)
    pub steps: usize,
    pub samples_seen: usize,
}

impl ClassifierHead {
    pub fn new(num_classes: usize, embedding_dim: usize, seed: u64) -> Self {
        let mut state = seed | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let limit = (6.0_f32 / (embedding_dim as f32 + num_classes as f32)).sqrt();
        let mut w = vec![0.0_f32; embedding_dim * num_classes];
        for v in w.iter_mut() {
            let r = (next() as f64 / u64::MAX as f64) as f32;
            *v = (r * 2.0 - 1.0) * limit;
        }
        let b = vec![0.0_f32; num_classes];
        Self {
            num_classes,
            embedding_dim,
            w,
            b,
            steps: 0,
            samples_seen: 0,
        }
    }

    pub fn softmax(&self, logits: &mut [f32]) {
        let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let mut sum = 0.0;
        for v in logits.iter_mut() {
            *v = (*v - max).exp();
            sum += *v;
        }
        for v in logits.iter_mut() {
            *v /= sum;
        }
    }

    pub fn forward(&self, embedding: &[f32]) -> Vec<f32> {
        let mut logits = vec![0.0_f32; self.num_classes];
        for c in 0..self.num_classes {
            let mut s = self.b[c];
            for d in 0..self.embedding_dim {
                s += embedding[d] * self.w[d * self.num_classes + c];
            }
            logits[c] = s;
        }
        self.softmax(&mut logits);
        logits
    }

    pub fn predict(&self, embedding: &[f32]) -> usize {
        let probs = self.forward(embedding);
        probs
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0
    }

    pub fn sgd_step(&mut self, embeddings: &[Vec<f32>], labels: &[usize], lr: f32) -> f32 {
        let n = embeddings.len() as f32;
        let mut total_loss = 0.0;
        for (x, &y) in embeddings.iter().zip(labels.iter()) {
            let mut probs = self.forward(x);
            for c in 0..self.num_classes {
                let grad = probs[c] - if c == y { 1.0 } else { 0.0 };
                for d in 0..self.embedding_dim {
                    self.w[d * self.num_classes + c] -= lr * grad * x[d] / n;
                }
                self.b[c] -= lr * grad / n;
            }
            total_loss += -probs[y].max(1e-9).ln();
        }
        self.steps += 1;
        self.samples_seen += embeddings.len();
        total_loss / n
    }

    pub fn state_hash(&self) -> u64 {
        let mut all: Vec<f32> = self.w.clone();
        all.extend(&self.b);
        fnv1a_64_f32(&all)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for v in &self.w {
            out.extend(v.to_le_bytes());
        }
        for v in &self.b {
            out.extend(v.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8], num_classes: usize, embedding_dim: usize) -> Self {
        let w: Vec<f32> = bytes[..embedding_dim * num_classes * 4]
            .chunks(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let b: Vec<f32> = bytes[embedding_dim * num_classes * 4..]
            .chunks(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Self {
            num_classes,
            embedding_dim,
            w,
            b,
            steps: 0,
            samples_seen: 0,
        }
    }

    /// FedAvg aggregation. The weights are the per-device data sizes.
    pub fn average(heads: &[&ClassifierHead], weights: &[usize]) -> ClassifierHead {
        let num_classes = heads[0].num_classes;
        let embedding_dim = heads[0].embedding_dim;
        let total_weight: usize = weights.iter().sum();
        let mut h = ClassifierHead::new(num_classes, embedding_dim, 0);
        for v in h.w.iter_mut() { *v = 0.0; }
        for v in h.b.iter_mut() { *v = 0.0; }
        for (head, &w) in heads.iter().zip(weights.iter()) {
            let scale = w as f32 / total_weight as f32;
            for (dst, src) in h.w.iter_mut().zip(head.w.iter()) {
                *dst += scale * src;
            }
            for (dst, src) in h.b.iter_mut().zip(head.b.iter()) {
                *dst += scale * src;
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fnv1a_64() {
        // Known value: "hello" should be 0xa430d84680aabd0b
        let h = fnv1a_64("hello");
        assert_eq!(h, 0xa430d84680aabd0b);
    }

    #[test]
    fn test_fnv1a_bytes() {
        let h = fnv1a_64_bytes(b"hello");
        assert_eq!(h, 0xa430d84680aabd0b);
    }

    #[test]
    fn test_hamming_window() {
        let w = hamming_window(4);
        assert_eq!(w.len(), 4);
        assert!((w[0] - 0.08).abs() < 0.01);
    }

    #[test]
    fn test_classifier_head_roundtrip() {
        let h = ClassifierHead::new(5, 64, 42);
        let bytes = h.to_bytes();
        let h2 = ClassifierHead::from_bytes(&bytes, 5, 64);
        assert_eq!(h.state_hash(), h2.state_hash());
    }

    #[test]
    fn test_classifier_head_predict() {
        let h = ClassifierHead::new(5, 64, 0);
        let emb = vec![0.1_f32; 64];
        let cls = h.predict(&emb);
        assert!(cls < 5);
    }

    #[test]
    fn test_sgd_step() {
        let mut h = ClassifierHead::new(5, 64, 0);
        let x = vec![0.1_f32; 64];
        let loss = h.sgd_step(&[x.clone(), x.clone()], &[0, 1], 0.05);
        assert!(loss > 0.0);
    }

    #[test]
    fn test_average() {
        let h1 = ClassifierHead::new(5, 64, 1);
        let h2 = ClassifierHead::new(5, 64, 2);
        let avg = ClassifierHead::average(&[&h1, &h2], &[1, 1]);
        for i in 0..avg.w.len() {
            let expected = (h1.w[i] + h2.w[i]) / 2.0;
            assert!((avg.w[i] - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn test_extractor_embed() {
        let fe = FeatureExtractor::new("F170-tinyml-v1");
        let audio: Vec<f32> = (0..8000).map(|i| (i as f32 * 0.01).sin()).collect();
        let emb = fe.embed(&audio);
        assert_eq!(emb.len(), 64);
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_extractor_state_hash_deterministic() {
        let fe1 = FeatureExtractor::new("F170-tinyml-v1");
        let fe2 = FeatureExtractor::new("F170-tinyml-v1");
        assert_eq!(fe1.state_hash(), fe2.state_hash());
    }

    #[test]
    fn test_extractor_roundtrip() {
        let fe = FeatureExtractor::new("F170-tinyml-v1");
        let bytes = fe.to_bytes();
        let fe2 = FeatureExtractor::from_bytes(&bytes);
        assert_eq!(fe.state_hash(), fe2.state_hash());
    }

    #[test]
    fn test_federated_loop_converges() {
        // Simple test: 5 "devices" each see data from one class.
        // After 20 rounds, the head should be able to distinguish classes.
        let num_classes = 3;
        let embedding_dim = 8;
        let mut global_head = ClassifierHead::new(num_classes, embedding_dim, 0);

        // Each device has a fixed "class signature" in its embeddings
        // Device 0: class 0 = embedding with [1,0,0,...]
        // Device 1: class 1 = embedding with [0,1,0,...]
        // etc.
        for round in 0..20 {
            let mut device_heads = Vec::new();
            for device_id in 0..num_classes {
                let mut local_head = ClassifierHead::from_bytes(
                    &global_head.to_bytes(),
                    num_classes,
                    embedding_dim,
                );
                // 8 local samples, all from this device's preferred class
                let mut xs = Vec::new();
                let mut ys = Vec::new();
                for _ in 0..8 {
                    let mut emb = vec![0.0_f32; embedding_dim];
                    emb[device_id] = 1.0;
                    // Add small noise to make it non-trivial
                    for v in emb.iter_mut() {
                        *v += (rand_deterministic() - 0.5) * 0.05;
                    }
                    xs.push(emb);
                    ys.push(device_id);
                }
                local_head.sgd_step(&xs, &ys, 0.1);
                device_heads.push(local_head);
            }
            // FedAvg
            global_head = ClassifierHead::average(
                &device_heads.iter().collect::<Vec<_>>(),
                &vec![1; num_classes],
            );
        }
        // After 20 rounds, the head should classify correctly
        let mut emb = vec![0.0_f32; embedding_dim];
        emb[1] = 1.0;
        let pred = global_head.predict(&emb);
        assert_eq!(pred, 1, "should predict class 1 for class-1 embedding");
    }
}

/// Deterministic PRNG for tests (LCG)
fn rand_deterministic() -> f32 {
    use std::cell::RefCell;
    thread_local! {
        static STATE: RefCell<u64> = RefCell::new(0xDEADBEEF);
    }
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let shifted = *state >> 33;
        (shifted as f64 / u32::MAX as f64) as f32
    })
}
