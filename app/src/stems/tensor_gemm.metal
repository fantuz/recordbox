#include <metal_stdlib>
#include <metal_tensor>
#include <MetalPerformancePrimitives/MetalPerformancePrimitives.h>
using namespace metal;
using namespace mpp::tensor_ops;

// c (M, N) = a (M, K) * b (K, N). a is stored M x K row-major, or K x M when TA; b is K x N,
// or N x K when TB. p = [M, N, K]. Thread groups of 4 SIMD groups, one TM x TN tile each.
template <int TM, int TN, bool TA, bool TB>
static inline void mm_f32(device float *a, device float *b, device float *c, constant uint *p, uint2 tgid) {
    int M = p[0], N = p[1], K = p[2];
    auto A = tensor<device float, dextents<int32_t, 2>, tensor_inline>(a, TA ? dextents<int32_t, 2>(M, K) : dextents<int32_t, 2>(K, M));
    auto B = tensor<device float, dextents<int32_t, 2>, tensor_inline>(b, TB ? dextents<int32_t, 2>(K, N) : dextents<int32_t, 2>(N, K));
    auto C = tensor<device float, dextents<int32_t, 2>, tensor_inline>(c, dextents<int32_t, 2>(N, M));
    constexpr auto desc = matmul2d_descriptor(TM, TN, static_cast<int>(dynamic_extent), TA, TB, false);
    matmul2d<desc, execution_simdgroups<4>> op;
    auto mA = TA ? A.slice(tgid.y * TM, 0) : A.slice(0, tgid.y * TM);
    auto mB = TB ? B.slice(0, tgid.x * TN) : B.slice(tgid.x * TN, 0);
    auto mC = C.slice(tgid.x * TN, tgid.y * TM);
    op.run(mA, mB, mC);
}
#define MM(name, TA, TB) \
kernel void name(device float *a [[buffer(0)]], device float *b [[buffer(1)]], device float *c [[buffer(2)]], \
                 constant uint *p [[buffer(3)]], uint2 tgid [[threadgroup_position_in_grid]]) { mm_f32<32, 128, TA, TB>(a, b, c, p, tgid); }
MM(mpp_mm_nn, false, false)
MM(mpp_mm_tn, true, false)
MM(mpp_mm_nt, false, true)
