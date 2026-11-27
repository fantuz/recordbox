//! The legacy analysis rekordbox's high precision analysis starts from.
//!
//! With high precision on, `CBeatAnalyzeThread::analyze` (0x1001528c4) switches its beat library
//! to `BeatAnalyzer_2_0` (`BALibWrapper::SetVersion(2)` when the analysis mode is 2..4), created
//! for constant tempo in Auto / Normal and for varied tempo in Dynamic, and feeds it the same
//! 16-bit samples as the standard analysis. `BeatAnalyzer_2_0::Analyzer::analyze(bool&)`
//! (0x10116be98) runs `BeatSensor`, `BeatInfoAnalyzer` and `UnitEnvelopeMaker`, and unlike
//! `BeatAnalyzer_1_0` no `UnitBeatAdjust` (2_0's `UnitBeatAdjust` only collects the envelopes
//! that `BeatInfoAnalyzer` and `analyzeFocus` read). `highPrecisionBeatAnalyze` (0x102327e50)
//! then reads `BA_GetBpm`, `BA_GetBeatNum`, `BA_GetOffsetCalc` (from the beat list) and, in Auto,
//! `GetBpmHendou`.
//!
//! The sensor, `UnitEnvelopeMaker`, gap correction and `findNearPos` of 2_0 are the 1_0
//! functions (masked byte compare; `BeatSensor::create` and `Analyzer::create` differ only in
//! the object layout), so those stages are recordbox's port of 1_0 (`rb_beat1`), run once for
//! both analyses when they share the same input. 2_0's own `BeatInfoAnalyzer::analyze` (0x101198e4c,
//! 52 KB) is `bia2`.

pub use super::super::rb_beat1::BeatInfo;
use super::super::rb_beat1::Front;

#[path = "rb_hp_bia2.rs"]
pub mod bia2;

/// `BeatAnalyzer_2_0` after `setSample`: the float BPM (`Analyzer` +72), the beat list (+88), the
/// tempo-change verdict (+45, `getBpmHendou`) and the flag `analyze(bool&)` hands back (set when
/// the tempo-change check halved a tempo above 180 BPM).
#[derive(Clone, Debug)]
pub struct Legacy2 {
    pub bpm: f32,
    pub beats: Vec<BeatInfo>,
    pub hendou: bool,
    pub halved: bool,
}

impl Legacy2 {
    /// `BA_GetBpm`: BPM x 100 rounded.
    pub fn bpm100(&self) -> i32 {
        self.bpm.mul_add(100.0, 0.5) as i32
    }
}

/// The legacy analysis of interleaved stereo at 44.1 or 48 kHz. `varied` = created for varied
/// tempo (Analysis Mode Dynamic); `low` = the low-band envelope of 2_0's `UnitBeatAdjust`
/// (`get_pdata_low`, `Uba::low`), which only exists when created for constant tempo.
pub(super) fn analyze(mut front: Front, sr: u32, varied: bool, low: Option<&[i16]>) -> Option<Legacy2> {
    let r = bia2::analyze(sr as i32, &front.list, front.total, if varied { None } else { low });
    // Residual phase jumps are an unresolved branch of the port. Preserve the
    // selected estimator's result when the reference verdict is unknown.
    if r.unported || !r.bpm.is_finite() || r.bpm <= 0.0 { return None; }
    let mut grid = r.list;
    front.adjust_envelope(r.bpm, &mut grid, varied);
    if grid.is_empty() {
        return None;
    }
    Some(Legacy2 { bpm: r.bpm, beats: grid, hendou: r.hendou, halved: r.halved })
}

#[cfg(test)]
mod tests {
    /// 2_0's verdict on two of the synthetic tempo programs, as rekordbox's own routine gives it
    /// (legacy2_oracle, PREPDECK_RBSYNTH=1): constant 124 BPM -> not varied, 124.00; a step from
    /// 128 to 86 BPM -> varied, the varied pass' 128.00.
    #[test]
    fn tempo_change_verdict_on_synthetic_programs() {
        use super::super::super::rb_beat1::{test_signals::{synth, SYNTH}, Front};
        for (k, want_hendou, want_bpm100) in [(0usize, false, 12400), (3, true, 12800)] {
            let (secs, f, seed, gap) = SYNTH[k];
            let samples = synth(secs, &f, seed, gap);
            let env = super::super::uba::Uba::from_audio(44100, &samples[..samples.len() - 2]);
            let l = super::analyze(Front::from_audio(&samples[..samples.len() - 2], 44_100).unwrap(), 44100, false, Some(&env.low)).unwrap();
            assert_eq!((l.hendou, l.bpm100()), (want_hendou, want_bpm100), "program {k}");
            // created for varied tempo there is no envelope: always varied
            let v = super::analyze(Front::from_audio(&samples[..samples.len() - 2], 44_100).unwrap(), 44100, true, None).unwrap();
            assert!(v.hendou, "program {k} varied");
        }
    }
}
