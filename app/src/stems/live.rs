//! A single separation worker publishes immutable, fully overlap-added samples. A
//! block is set once and kept alive by the deck's Arc. The audio callback neither
//! waits for the worker nor observes a partially filled plane.
use super::{Hardware, ModelSpec, SeparationDevice};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, OnceLock,
};

const BLOCK: usize = 8192;
const PLANES: usize = 8;
const GIB: u64 = 1 << 30;
const MAX_BYTES: usize = 512 << 20;

#[derive(Debug)]
pub struct Live {
    len: usize,
    limit: usize,
    blocks: Box<[OnceLock<Box<[f32]>>]>,
    ready: AtomicUsize,
    exhausted: AtomicBool,
}

impl Live {
    /// Static eligibility for the Preferences screen. Query the current device budget
    /// separately when starting the job; the driver can change its budget at any time.
    pub fn supported(hw: &Hardware, model: &ModelSpec, execution: SeparationDevice) -> bool {
        if !hw.ram_gb.is_finite()
            || !hw.gpu
            || execution == SeparationDevice::Cpu
            || super::backend_hold().is_some()
            || !matches!(model.id, "htdemucs" | "htdemucs_ft")
        {
            return false;
        }
        static CAPABLE: OnceLock<bool> = OnceLock::new();
        *CAPABLE.get_or_init(|| device_budget(hw).is_some())
    }

    /// Both the completed output and the input already belong to the normal job.
    /// This cap covers all additional sample storage retained by this preview.
    pub fn for_job(
        hw: &Hardware,
        model: &ModelSpec,
        execution: SeparationDevice,
        source_frames: usize,
        source_rate: u32,
    ) -> Option<Arc<Self>> {
        if !Self::supported(hw, model, execution) || source_rate == 0 {
            return None;
        }
        let (available, required) = device_budget(hw)?;
        if available < required {
            return None;
        }
        let len = (source_frames as f64 * super::htdemucs::SAMPLE_RATE as f64 / source_rate as f64)
            .round() as usize;
        Self::with_len(len).map(Arc::new)
    }

    pub(super) fn with_len(len: usize) -> Option<Self> {
        Self::with_frame_limit(len, MAX_BYTES / (PLANES * size_of::<f32>()))
    }

    fn with_frame_limit(len: usize, cap: usize) -> Option<Self> {
        if len == 0 {
            return None;
        }
        len.checked_mul(PLANES * size_of::<f32>())?;
        let limit = len.min(cap);
        let count = limit.div_ceil(BLOCK);
        let mut blocks = Vec::new();
        blocks.try_reserve_exact(count).ok()?;
        blocks.resize_with(count, OnceLock::new);
        Some(Self {
            len,
            limit,
            blocks: blocks.into_boxed_slice(),
            ready: 0.into(),
            exhausted: false.into(),
        })
    }

    /// An ordinary published prefix with deterministic samples for deck lifecycle tests.
    #[cfg(test)]
    pub(crate) fn test_prefix(len: usize, planes: &[f32], until: usize) -> Arc<Self> {
        let live = Arc::new(Self::with_len(len).unwrap());
        assert_eq!(planes.len(), len * PLANES);
        assert!(until <= len);
        Publication {
            live: &live,
            previous: &[],
        }
        .advance(planes, &vec![1.0; len], until);
        live
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn ready_frames(&self) -> usize {
        self.ready.load(Ordering::Acquire)
    }

    /// A published range within a single slab. The range is half-open, in 44.1 kHz frames.
    #[inline]
    pub fn plane_span(&self, plane: usize, start: usize, end: usize) -> Option<&[f32]> {
        if plane >= PLANES || start >= end || end > self.limit || start / BLOCK != (end - 1) / BLOCK
        {
            return None;
        }
        let index = start / BLOCK;
        let data = self.blocks.get(index)?.get()?;
        let base = plane * (data.len() / PLANES) + start % BLOCK;
        Some(&data[base..base + end - start])
    }

    #[inline]
    pub fn sample(&self, plane: usize, frame: usize) -> Option<f32> {
        self.plane_span(plane, frame, frame.checked_add(1)?)
            .map(|s| s[0])
    }
}

/// A worker on the final model writes all eight planes, in track order. For the
/// fine-tuned set six planes have already been normalized by previous passes.
#[derive(Clone, Copy)]
pub(super) struct Publication<'a> {
    pub live: &'a Live,
    pub previous: &'a [f32],
}

impl Publication<'_> {
    /// Segments that start at or after `until` cannot contribute to any frame before
    /// it. Publish complete blocks there, retaining the remainder for the next segment.
    pub fn advance(self, current: &[f32], weights: &[f32], until: usize) {
        debug_assert_eq!(weights.len(), self.live.len);
        self.publish_blocks(current, until, |start, end, den| {
            den.copy_from_slice(&weights[start..end])
        });
    }

    /// The accelerated pipeline keeps its normalizer until after inference. For a
    /// preview reproduce its exact addition order only for each block being copied.
    #[cfg(any(test, target_os = "macos", feature = "directml"))]
    pub fn advance_segments(
        self,
        current: &[f32],
        until: usize,
        offsets: &[usize],
        weight: &[f32],
    ) {
        self.publish_blocks(current, until, |start, end, den| {
            den.fill(0.0);
            let first = offsets.partition_point(|&o| o.saturating_add(weight.len()) <= start);
            for &o in offsets[first..].iter().take_while(|&&o| o < end) {
                let lo = start.max(o);
                let hi = end.min(o + weight.len());
                for (v, w) in den[lo - start..hi - start]
                    .iter_mut()
                    .zip(&weight[lo - o..hi - o])
                {
                    *v += w;
                }
            }
        });
    }

    fn publish_blocks(
        self,
        current: &[f32],
        until: usize,
        mut weights: impl FnMut(usize, usize, &mut [f32]),
    ) {
        let live = self.live;
        let until = until.min(live.limit);
        let mut start = live.ready.load(Ordering::Relaxed);
        if start >= until || live.exhausted.load(Ordering::Relaxed) {
            return;
        }
        let len = live.len;
        debug_assert_eq!(current.len() + self.previous.len(), PLANES * len);
        let mut den = [0.0; BLOCK];
        while start < until {
            let end = (start + BLOCK).min(live.limit);
            if end > until {
                break;
            }
            let frames = end - start;
            let mut data = Vec::new();
            if data.try_reserve_exact(PLANES * frames).is_err() {
                live.exhausted.store(true, Ordering::Relaxed);
                return;
            }
            for plane in self.previous.chunks_exact(len) {
                data.extend_from_slice(&plane[start..end]);
            }
            weights(start, end, &mut den[..frames]);
            for plane in current.chunks_exact(len) {
                data.extend(
                    plane[start..end]
                        .iter()
                        .zip(&den[..frames])
                        .map(|(v, w)| v / w.max(1e-9)),
                );
            }
            if live.blocks[start / BLOCK]
                .set(data.into_boxed_slice())
                .is_err()
            {
                return;
            }
            live.ready.store(end, Ordering::Release);
            start = end;
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn device_budget(hw: &Hardware) -> Option<(u64, u64)> {
    if !hw.apple_silicon || hw.ram_gb * 1e9 < (32 * GIB) as f64 {
        return None;
    }
    let device = super::device();
    let gpu = device.as_metal_device().ok()?.metal_device();
    let budget = gpu.recommended_max_working_set_size();
    if budget < (8 * GIB) as usize {
        return None;
    }
    let allocated = gpu.current_allocated_size();
    Some((budget.saturating_sub(allocated) as u64, 8 * GIB))
}

#[cfg(all(
    feature = "cuda12",
    not(any(
        target_os = "macos",
        all(windows, feature = "directml"),
        all(target_os = "linux", feature = "vulkan")
    ))
))]
fn device_budget(hw: &Hardware) -> Option<(u64, u64)> {
    if hw.ram_gb * 1e9 < (24 * GIB) as f64 {
        return None;
    }
    let device = super::device();
    let stream = device.as_cuda_device().ok()?.cuda_stream();
    let context = stream.context();
    let (major, _) = context.compute_capability().ok()?;
    let (free, total) = context.mem_get_info().ok()?;
    if major < 8 || (total as u64) < 8 * GIB {
        return None;
    }
    Some((free as u64, 6 * GIB))
}

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(
        feature = "cuda12",
        not(any(
            target_os = "macos",
            all(windows, feature = "directml"),
            all(target_os = "linux", feature = "vulkan")
        ))
    )
)))]
fn device_budget(_: &Hardware) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_normalization_matches_completed_stems_in_both_model_modes() {
        let len = 3 * BLOCK + 67;
        let weights: Vec<_> = (0..len).map(|f| ((f % 43) + 1) as f32 / 47.0).collect();
        let weighted: Vec<_> = (0..PLANES * len)
            .map(|i| (i % 97) as f32 * weights[i % len])
            .collect();
        let complete: Vec<_> = weighted
            .iter()
            .enumerate()
            .map(|(i, w)| w / weights[i % len])
            .collect();
        for previous_planes in [0, 6] {
            let live = Live::with_len(len).unwrap();
            let previous = &complete[..previous_planes * len];
            let partial = &weighted[previous_planes * len..];
            let publish = Publication {
                live: &live,
                previous,
            };
            for boundary in [13, BLOCK + 5, 2 * BLOCK + 15, len] {
                publish.advance(partial, &weights, boundary);
                let n = live.ready_frames();
                assert_eq!(
                    n,
                    if boundary == len {
                        len
                    } else {
                        boundary / BLOCK * BLOCK
                    }
                );
                assert_eq!(live.sample(0, n), None);
                for plane in 0..PLANES {
                    for frame in 0..n {
                        assert_eq!(
                            live.sample(plane, frame),
                            Some(complete[plane * len + frame])
                        );
                    }
                }
            }
            assert_eq!(live.plane_span(3, BLOCK - 1, BLOCK + 1), None);
            assert_eq!(
                live.plane_span(7, len - 2, len).unwrap(),
                &complete[PLANES * len - 2..]
            );
        }
    }

    #[test]
    fn a_long_track_stops_publishing_at_the_limit_and_preserves_plane_stride() {
        let len = 4 * BLOCK;
        let cap = 2 * BLOCK + 7;
        let live = Live::with_frame_limit(len, cap).unwrap();
        let out: Vec<_> = (0..PLANES * len).map(|i| i as f32).collect();
        let weights = vec![1.; len];
        let publish = Publication {
            live: &live,
            previous: &[],
        };
        for until in [BLOCK, 3 * BLOCK, len] {
            publish.advance(&out, &weights, until);
        }
        assert_eq!(live.ready_frames(), cap);
        assert!(live.plane_span(2, cap - 1, cap + 1).is_none());
        for plane in 0..PLANES {
            assert_eq!(
                live.sample(plane, cap - 1),
                Some(out[plane * len + cap - 1])
            );
            assert_eq!(live.sample(plane, cap), None);
        }
    }

    #[test]
    fn bounded_storage_and_safe_source_edges() {
        assert!(Live::with_len(0).is_none());
        // The full logical track can be hours long; only the prefix's slab
        // pointers are reserved and an unready later frame has no audio.
        let limit = MAX_BYTES / 32;
        let long = Live::with_len(limit * 20).unwrap();
        assert_eq!(long.len(), limit * 20);
        assert_eq!(long.limit, limit);
        assert_eq!(long.blocks.len(), limit.div_ceil(BLOCK));
        assert_eq!(long.ready_frames(), 0);
        assert_eq!(long.sample(0, limit + 5), None);
        assert!(Live::with_len(usize::MAX).is_none());
        let live = Live::with_len(1).unwrap();
        assert_eq!(live.sample(0, 0), None);
        Publication {
            live: &live,
            previous: &[],
        }
        .advance(&[2.0; PLANES], &[2.0], 1);
        assert_eq!(live.sample(7, 0), Some(1.0));
        assert_eq!(live.sample(8, 0), None);
        assert_eq!(live.sample(0, usize::MAX), None);
    }
}
