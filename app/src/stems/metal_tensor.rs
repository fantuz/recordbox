//! Public Metal 4 tensor operations for eligible HTDemucs matrix multiplies.

use super::{alloc, msg};
use candle_core::backend::BackendStorage;
use candle_core::{CpuStorage, CustomOp2, DType, Device, Layout, MetalDevice, MetalStorage, Result, Shape, Tensor};
use candle_metal_kernels::metal::ComputePipeline;
use objc2_metal::{
    MTLCompileOptions, MTLComputePipelineState, MTLDevice, MTLGPUFamily, MTLLanguageVersion, MTLMathMode, MTLSize,
};
use std::sync::{Arc, Mutex};

const ROWS: usize = 32;
const COLS: usize = 128;
const KERNELS: [&str; 3] = ["mpp_mm_nn", "mpp_mm_tn", "mpp_mm_nt"];

/// Select only complete output tiles, dense f32 matrices and zero offsets. Inputs may have
/// an odd inner dimension (HTDemucs appends a bias row). Every index in MSL fits int32.
fn variant(left: &Layout, right: &Layout) -> Option<usize> {
    let (&[m, k], &[k2, n]) = (left.dims(), right.dims()) else {
        return None;
    };
    if m == 0
        || n == 0
        || k == 0
        || k != k2
        || !m.is_multiple_of(ROWS)
        || !n.is_multiple_of(COLS)
        || left.start_offset() != 0
        || right.start_offset() != 0
    {
        return None;
    }
    for length in [m.checked_mul(k)?, k.checked_mul(n)?, m.checked_mul(n)?] {
        i32::try_from(length).ok()?;
    }
    match (left.stride(), right.stride()) {
        (l, r) if l == [k, 1] && r == [n, 1] => Some(0),
        (l, r) if l == [1, m] && r == [n, 1] => Some(1),
        (l, r) if l == [k, 1] && r == [1, k] => Some(2),
        _ => None,
    }
}

pub(super) fn multiply(a: &Tensor, b: &Tensor) -> Option<Result<Tensor>> {
    let Device::Metal(dev) = a.device() else { return None };
    if a.dtype() != DType::F32 || b.dtype() != DType::F32 || !a.device().same_device(b.device()) {
        return None;
    }
    let index = variant(a.layout(), b.layout())?;
    let pipelines = pipelines(dev)?;
    Some(a.apply_op2_no_bwd(b, &pipelines[index]))
}

type Adapters = Vec<(u64, Option<Arc<[TensorGemm; 3]>>)>;

fn pipelines(dev: &MetalDevice) -> Option<Arc<[TensorGemm; 3]>> {
    static ADAPTERS: Mutex<Adapters> = Mutex::new(Vec::new());
    let mut adapters = ADAPTERS.lock().ok()?;
    let id = dev.metal_device().registry_id();
    if let Some((_, known)) = adapters.iter().find(|(adapter, _)| *adapter == id) {
        return known.clone();
    }
    let gpu = dev.metal_device().as_ref();
    let disabled = std::env::var_os("RECORDBOX_METAL_TENSOR_OPS").is_some_and(|v| v == "0");
    let kernels = if !disabled && gpu.supportsFamily(MTLGPUFamily::Apple10) && gpu.supportsFamily(MTLGPUFamily::Metal4)
    {
        match compile_and_verify(dev) {
            Ok(ops) => Some(Arc::new(ops)),
            Err(e) => {
                eprintln!("Metal tensor operations unavailable, using Candle matrix multiplication: {e}");
                None
            }
        }
    } else {
        None
    };
    adapters.push((id, kernels.clone()));
    kernels
}

struct TensorGemm {
    pipeline: ComputePipeline,
    variant: usize,
}

impl CustomOp2 for TensorGemm {
    fn name(&self) -> &'static str {
        KERNELS[self.variant]
    }

    fn cpu_fwd(&self, _: &CpuStorage, _: &Layout, _: &CpuStorage, _: &Layout) -> Result<(CpuStorage, Shape)> {
        candle_core::bail!("tensor GEMM requires Metal")
    }

    fn metal_fwd(
        &self,
        a: &MetalStorage,
        left: &Layout,
        b: &MetalStorage,
        right: &Layout,
    ) -> Result<(MetalStorage, Shape)> {
        if variant(left, right) != Some(self.variant) || a.dtype() != DType::F32 || b.dtype() != DType::F32 {
            candle_core::bail!("unsupported Metal tensor layout")
        }
        let (m, k) = left.shape().dims2()?;
        let n = right.dims()[1];
        if a.buffer().length() / 4 < m * k || b.buffer().length() / 4 < k * n {
            candle_core::bail!("Metal tensor inputs exceed their buffers")
        }
        let dev = a.device().clone();
        let output = alloc(&dev, m * n)?;
        let command = dev.command_encoder()?;
        let enc = command.as_ref();
        enc.set_label(self.name());
        enc.set_compute_pipeline_state(&self.pipeline);
        enc.set_input_buffer(0, Some(a.buffer()), 0);
        enc.set_input_buffer(1, Some(b.buffer()), 0);
        enc.set_output_buffer(2, Some(&output), 0);
        enc.set_bytes(3, &[m as u32, n as u32, k as u32]);
        enc.dispatch_thread_groups(
            MTLSize {
                width: n / COLS,
                height: m / ROWS,
                depth: 1,
            },
            MTLSize {
                width: 128,
                height: 1,
                depth: 1,
            },
        );
        drop(command);
        Ok((MetalStorage::new(output, dev, m * n, DType::F32), Shape::from((m, n))))
    }
}

fn compile_and_verify(dev: &MetalDevice) -> Result<[TensorGemm; 3]> {
    let gpu = dev.metal_device();
    let options = MTLCompileOptions::new();
    options.setLanguageVersion(MTLLanguageVersion::Version4_0);
    options.setMathMode(MTLMathMode::Safe);
    let library = gpu
        .new_library_with_source(include_str!("tensor_gemm.metal"), Some(&options))
        .map_err(msg)?;
    let compile = |variant: usize| -> Result<TensorGemm> {
        let function = library.get_function(KERNELS[variant], None).map_err(msg)?;
        let pipeline = gpu.new_compute_pipeline_state_with_function(&function).map_err(msg)?;
        if pipeline.as_ref().threadExecutionWidth() != 32 || pipeline.max_total_threads_per_threadgroup() < 128 {
            candle_core::bail!("incompatible Metal tensor threadgroup")
        }
        Ok(TensorGemm { pipeline, variant })
    };
    let ops = [compile(0)?, compile(1)?, compile(2)?];
    verify(&Device::Metal(dev.clone()), &ops)?;
    Ok(ops)
}

/// One tile with an odd inner dimension, in each permitted layout. A failed numerical
/// check disables this adapter's tensor path for the process before it handles any audio.
fn verify(device: &Device, ops: &[TensorGemm; 3]) -> Result<()> {
    const K: usize = 65;
    let sample = |i: usize| {
        let bits = (i as u32).wrapping_mul(2_654_435_761).rotate_left(11);
        (bits >> 8) as f32 / 8_388_608.0 - 1.0
    };
    let left: Vec<_> = (0..ROWS * K).map(sample).collect();
    let right: Vec<_> = (0..K * COLS).map(|i| sample(i + 983)).collect();
    let mut expected = Vec::with_capacity(ROWS * COLS);
    for row in left.as_chunks::<K>().0 {
        for col in 0..COLS {
            expected.push(
                (0..K)
                    .map(|j| row[j] as f64 * right[j * COLS + col] as f64)
                    .sum::<f64>(),
            );
        }
    }
    let signal: f64 = expected.iter().map(|v| v * v).sum();
    let a = Tensor::from_vec(left, (ROWS, K), device)?;
    let b = Tensor::from_vec(right, (K, COLS), device)?;
    let at = a.t()?.contiguous()?.t()?;
    let bt = b.t()?.contiguous()?.t()?;
    for ((left, right), op) in [(&a, &b), (&at, &b), (&a, &bt)].into_iter().zip(ops) {
        let output = left.apply_op2_no_bwd(right, op)?.flatten_all()?.to_vec1::<f32>()?;
        let error: f64 = output
            .iter()
            .zip(&expected)
            .map(|(actual, want)| (*actual as f64 - want).powi(2))
            .sum();
        if !error.is_finite() || error > signal * 1e-10 {
            candle_core::bail!("{} failed its f32 numerical check", op.name())
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tensor_tiles_require_supported_shapes_and_views() {
        // Real transformer geometry, including its bias row.
        let left = Layout::contiguous((2688, 513));
        let right = Layout::contiguous((513, 2048));
        let lt = Layout::new((2688, 513).into(), vec![1, 2688], 0);
        let rt = Layout::new((513, 2048).into(), vec![1, 513], 0);
        assert_eq!(variant(&left, &right), Some(0));
        assert_eq!(variant(&lt, &right), Some(1));
        assert_eq!(variant(&left, &rt), Some(2));
        assert_eq!(variant(&lt, &rt), None);
        for left in [
            Layout::contiguous((2687, 513)),
            Layout::contiguous((2688, 0)),
            Layout::contiguous((2, 2688, 513)),
            Layout::contiguous_with_offset((2688, 513), 16),
            Layout::new((2688, 513).into(), vec![514, 1], 0),
            Layout::contiguous((1usize << 29, 513)),
        ] {
            assert_eq!(variant(&left, &right), None);
        }
        assert_eq!(variant(&left, &Layout::contiguous((513, 2047))), None);
        assert_eq!(variant(&left, &Layout::contiguous((512, 2048))), None);
    }

    #[test]
    fn tensor_gemm_matches_reference_on_apple10() -> Result<()> {
        let Ok(Device::Metal(dev)) = Device::new_metal(0) else {
            eprintln!("Metal tensor hardware check skipped: no Metal device on this runner");
            return Ok(());
        };
        let gpu = dev.metal_device().as_ref();
        if gpu.supportsFamily(MTLGPUFamily::Apple10) && gpu.supportsFamily(MTLGPUFamily::Metal4) {
            compile_and_verify(&dev)?;
            eprintln!("Metal 4 tensor GEMM verified all three layouts on Apple GPU family 10");
        } else {
            eprintln!("Metal tensor hardware check skipped: the device does not expose Apple10 and Metal 4");
        }
        Ok(())
    }
}
