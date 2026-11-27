//! Stage D driver of the phrase analyser port (`Sub_Main_D`): from stage B's and C's results to
//! the phrase list. Every analyser call can be recorded (`Trace`) so a comparison run can match
//! it against the reference's call-by-call dump.

use crate::phrase_rb_c::{Mat2, StageC};
use crate::phrase_rb_d1 as d1;
use crate::phrase_rb_d2 as d2;
use crate::phrase_rb_d3 as d3;
use crate::phrase_rb_d4 as d4;
use crate::phrase_rb_d5 as d5;
use crate::phrase_rb_d6 as d6;
use crate::phrase_rb_d7 as d7;
use crate::phrase_rb_d8 as d8;
use crate::phrase_rb_d9 as d9;
use crate::phrase_rb_d10 as d10;
use crate::phrase_rb_d11 as d11;
use crate::phrase_rb_d12 as d12;
use crate::phrase_rb_d13 as d13;
use crate::phrase_rb_d14 as d14;

/// One recorded call: name and its argument values after the call (by argument index of the
/// reference's prototype, `self` = 0).
#[derive(Clone, Debug, Default)]
pub struct Call {
    pub name: &'static str,
    pub args: Vec<(usize, Vec<f64>)>,
}

#[derive(Default)]
pub struct Trace {
    pub on: bool,
    pub calls: Vec<Call>,
}

impl Trace {
    pub fn rec(&mut self, name: &'static str, args: impl FnOnce() -> Vec<(usize, Vec<f64>)>) {
        if self.on {
            self.calls.push(Call { name, args: args() });
        }
    }
}

pub fn fv(x: &[f64]) -> Vec<f64> {
    x.to_vec()
}
pub fn iv(x: &[i32]) -> Vec<f64> {
    x.iter().map(|v| *v as f64).collect()
}
pub fn uv(x: &[u8]) -> Vec<f64> {
    x.iter().map(|v| *v as f64).collect()
}
pub fn sv(x: f64) -> Vec<f64> {
    vec![x]
}

/// What stage D reads from the analyser object (after stages B and C).
pub struct DIn<'a> {
    pub c: &'a StageC,
    pub b: &'a crate::phrase_rb_b::StageB,
    pub bpm: f64,
    pub sr: f64,
    pub sr8: f64,
    pub total: i32,
    pub ms_in: i32,
    pub key: i32,
}

/// The result as `SongStructureAnalyzeResult` holds it.
#[derive(Clone, Debug, Default)]
pub struct DOut {
    pub n: i32,
    pub first_ms: i32,
    pub mood: i32,
    pub bar: i32,
    pub h4: i32,
    pub h5: i32,
    pub end: i32,
    /// beat, kind, k1, k2, b, beat2, beat3, beat4, k3, fill, fill beat
    pub rows: Vec<[i32; 11]>,
}

#[cfg(test)]
#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
pub fn sub_main_d(inp: &DIn, tr: &mut Trace) -> Option<DOut> {
    sub_main_d_cancellable(inp, tr, &|| false)
}

pub fn sub_main_d_cancellable(inp: &DIn, tr: &mut Trace, cancelled: &dyn Fn() -> bool) -> Option<DOut> {
    if cancelled() { return None; }
    let c = inp.c;
    let v2 = inp.bpm;
    let v6 = c.nbeats;
    let p1 = inp.b.first_bar;
    if c.zero_flag != 1 {
        return Some(DOut { n: 0, ..Default::default() });
    }
    let mut chroma = c.g3568.clone();
    let (p6, p7, selfa) = d1::intro_end_find_b(&mut chroma, c.f888, c.f889, v6);
    tr.rec("IntroEndFind_b", || vec![(3, chroma.d.clone()), (7, uv(&p6)), (8, uv(&p7)), (9, fv(&selfa))]);
    if cancelled() { return None; }
    let (p2, p3) = d1::low_level_b(&c.g3224, p1);
    tr.rec("LowLevelAnalyze_b", || vec![(3, fv(&p2)), (4, uv(&p3))]);
    let (v240, v290) = d1::middle_level_b(&c.g3256, p1);
    tr.rec("MiddleLevelAnalyze_b", || vec![(3, uv(&v240)), (4, fv(&v290))]);
    let mut mp = c.g3160.clone();
    let v289 = d1::middle_peak_b(&mut mp, v6, p1);
    tr.rec("MiddlePeakAnalyze_b", || vec![(1, fv(&mp)), (4, fv(&v289))]);
    let (v288, v287) = d1::all_level_b(&c.g3384, p1);
    tr.rec("AllLevelAnalyze_b", || vec![(3, fv(&v288)), (4, fv(&v287))]);
    let v239 = d1::hihat_b(&c.g3512, p1);
    tr.rec("HiHatAnalyze_b", || vec![(3, uv(&v239))]);
    let (v286, v237) = d1::high_peak32_b(&c.g3416, p1)?;
    tr.rec("HighPeakAnalyze32_b", || vec![(3, fv(&v286)), (4, sv(v237 as f64))]);
    let v285 = d1::uta_umu(&c.g3616);
    tr.rec("UtaUmuAnalyze_a", || vec![(2, fv(&v285))]);
    let rh = &c.g3448[..c.n3448.min(c.g3448.len())];
    let rh2 = &c.g3480[..c.n3480.min(c.g3480.len())];
    let (p5, p17) = d2::detect_comp(&c.g2968, &c.g3000, rh, p1);
    tr.rec("DetectComp", || vec![(6, sv(p5)), (7, sv(p17))]);
    let v236 = d2::detect_fade_out(&c.g2968, &c.g3000);
    tr.rec("DetectFadeOut", || vec![(4, sv(v236 as f64))]);
    let p10 = d2::calc_max_level_ratio(&c.g3032);
    tr.rec("CalcMaxLevelRatio", || vec![(2, sv(p10))]);
    let (p12, p13) = d2::detect_bpm128(v2);
    tr.rec("DetectBPM128", || vec![(2, sv(p12 as f64)), (3, sv(p13 as f64))]);
    let v233 = d2::detect_tenkei_genre1(v2, inp.b.f520);
    tr.rec("DetectTenkeiGenre1", || vec![(3, sv(v233 as f64))]);
    let v211 = d2::detect_slowrock(rh, rh2);
    tr.rec("DetectSlowrock", || vec![(4, sv(v211 as f64))]);
    let (v209, p4, v283, v208, p15) = d2::calc_track_low_power_ratio(&c.g2968, &c.g3000, v6);
    tr.rec("CalcTrackLowPowerRatio", || vec![(4, sv(v209)), (5, fv(&p4)), (6, fv(&v283)), (7, sv(v208)), (8, sv(p15))]);
    let (v184, v188, v187, v183, p16) = d2::calc_track_low_power_ratio(&c.g3352, &c.g3000, v6);
    tr.rec("CalcTrackLowPowerRatio", || vec![(4, sv(v184)), (5, fv(&v188)), (6, fv(&v187)), (7, sv(v183)), (8, sv(p16))]);
    if cancelled() { return None; }
    let (p11, v182) = d2::detect_slow_tempo(inp.sr8, &v283, v2, &inp.b.m528, inp.ms_in, inp.b.first_ms, &c.bt8, v6, inp.b.f592, inp.b.f596);
    tr.rec("DetectSlowTempo", || vec![(12, sv(p11 as f64)), (13, sv(v182 as f64))]);
    let (v215, v214, v213, v212) = d2::four_kicks_full(rh, v2, &v283);
    tr.rec("Detect4KicksPopRockRhythm", || vec![(4, sv(v215 as f64)), (5, sv(v214 as f64)), (6, sv(v213 as f64)), (7, sv(v212 as f64))]);
    if cancelled() { return None; }
    let mut mood = d2::genre_devide_overall(v211, p11, v209, v214, v213, v212, v215, v236, 0, 0, p12, p13, v233, p17);
    tr.rec("GenreDevideOverall", || vec![(19, sv(mood as f64))]);
    d2::genre12_final_judge(&mut mood, v6, &c.g3064, &c.g3128);
    tr.rec("Genre12FinalJudge", || vec![(1, sv(mood as f64))]);
    let v266 = d2::find_secure_point(&c.g3064, p1)?;
    tr.rec("FindSecurePoint", || vec![(3, iv(&v266))]);
    // secure points as beats (v264), and those within the first 199 beats (v265)
    let mut v264: Vec<i32> = if v266[0] != 0 { v266.iter().map(|x| p1 - 4 + 4 * x).collect() } else { vec![0; v266.len()] };
    let v262 = v264.clone();
    let mut v265 = v264.clone();
    for x in v265.iter_mut() {
        if ((*x).wrapping_sub(200)) as u32 <= 0xFFFFFF38 {
            *x = 0;
        }
    }
    if v266[0] != 0 {
        let mut k = v265.len();
        while k > 0 {
            k -= 1;
            if v265[k] == 0 {
                v265.remove(k);
            }
        }
    }
    let m = (2 * v266.len()) | 1;
    let rh2 = &c.g3480[..c.n3480.min(c.g3480.len())];
    let p18_0;
    if mood == 1 {
        p18_0 = 1;
    } else if mood != 2 {
        p18_0 = 2;
    } else {
        let (mut v261, mut v260, mut v251, mut v250) = (vec![0i32; m], vec![0i32; m], vec![0i32; m], vec![0i32; m]);
        if v266[0] != 0 {
            let s0 = v266[0];
            v261[0] = 1;
            v260[0] = 1;
            v251[0] = 10;
            v250[0] = 10;
            let (mut v58, mut v59);
            if s0 < 10 {
                v59 = 0usize;
                v261[1] = p1 + 4 * s0 - 4;
                v58 = 1usize;
                v251[1] = 40;
            } else {
                v58 = 0;
                v261[1] = p1 + 4 * s0 - 36;
                v261[2] = p1 + 4 * s0 - 4;
                v251[2] = 40;
                v59 = 1;
                v251[1] = 30;
            }
            let (mut v167, mut v166);
            if s0 < 18 {
                v260[1] = p1 + 4 * s0 - 4;
                v166 = 0usize;
                v167 = 1usize;
                v250[1] = 40;
            } else {
                v260[1] = p1 + 4 * s0 - 68;
                v260[2] = p1 + 4 * s0 - 4;
                v250[2] = 40;
                v166 = 1;
                v167 = 0;
                v250[1] = 30;
            }
            for k in 0..v266.len().saturating_sub(1) {
                let v81 = v266[k + 1];
                if v81 - v266[k] < 9 {
                    v58 += 1;
                    let i = v58 + 2 * v59;
                    v261[i] = p1 - 4 + 4 * v81;
                    v251[i] = 40;
                } else {
                    let i = v58 + 2 * v59;
                    v261[i + 1] = p1 - 36 + 4 * v81;
                    v261[i + 2] = p1 - 4 + 4 * v81;
                    v251[i + 1] = 30;
                    v251[i + 2] = 40;
                    v59 += 1;
                }
            }
            for k in (0..v261.len()).rev() {
                if v261[k] == 0 {
                    v261.remove(k);
                    v251.remove(k);
                }
            }
            for k in 0..v266.len().saturating_sub(1) {
                let v81 = v266[k + 1];
                if v81 - v266[k] < 17 {
                    v167 += 1;
                    let i = v167 + 2 * v166;
                    v260[i] = p1 - 4 + 4 * v81;
                    v250[i] = 40;
                } else {
                    let i = v167 + 2 * v166;
                    v260[i + 1] = p1 - 68 + 4 * v81;
                    v260[i + 2] = p1 - 4 + 4 * v81;
                    v250[i + 1] = 30;
                    v250[i + 2] = 40;
                    v166 += 1;
                }
            }
            for k in (0..v260.len()).rev() {
                if v260[k] == 0 {
                    v260.remove(k);
                    v250.remove(k);
                }
            }
            let (v270, v269, v268) = d4::block_average(&v261, &selfa, &c.g3064, p1, &c.g3128)?;
            tr.rec("BlockAverage", || vec![(7, fv(&v270)), (8, fv(&v269)), (9, fv(&v268))]);
            let (v249, v267) = d4::up_simpleloop(&v261, &mut v251, &c.g3288, &c.g3320, &v269)?;
            tr.rec("UpSimpleloopAna", || vec![(2, iv(&v251)), (7, iv(&v249)), (8, fv(&v267))]);
            let (k, v194, v197, v196, v195) = d4::up4_8_16(&v260, &v250, rh2)?;
            v250 = k;
            tr.rec("Up4_8_16Ana", || vec![(2, iv(&v250)), (5, iv(&v194)), (6, iv(&v197)), (7, iv(&v196)), (8, iv(&v195))]);
        }
        let dur = inp.total as f64 / inp.sr;
        let (mo, ty) = d4::type_genre_judge(&c.g3064, p1, dur, &v251, &v250, mood)?;
        mood = mo;
        p18_0 = ty;
        tr.rec("SongStructureAna_Type_Genre_Judge", || vec![(7, sv(mood as f64)), (8, sv(p18_0 as f64))]);
    }
    if cancelled() { return None; }
    let (v259, v231, v230) = d3::intro_end_judge_main(&c.g3064, &p2, &v289, p1, &v285, &v265)?;
    tr.rec("IntroEndJudge_main", || vec![(7, iv(&v259)), (8, sv(v231 as f64)), (9, sv(v230 as f64))]);
    let vh = d5::verse_hook_ana_data23(&c.g3064, &p2, &mp, &v289, p1)?;
    tr.rec("VerseHookAnaData23", || {
        let mut a = vec![];
        for (g, r) in vh.iter().enumerate() {
            a.push((6 + 3 * g, fv(&r.v)));
            a.push((7 + 3 * g, fv(&r.fold)));
            a.push((8 + 3 * g, fv(&r.sign)));
        }
        a
    });
    if cancelled() { return None; }
    let v181 = inp.b.f600;
    let mut v200: Vec<i32> = vec![];
    if p18_0 == 1 {
        if v230 == 1 {
            let (v229, v258) = d5::verse_hook_judge_lowpeak(v231, &c.g3096, p1, &p3)?;
            tr.rec("VerseHookJudge_LowPeak", || vec![(5, sv(v229 as f64)), (6, iv(&v258))]);
            let v191: Vec<i32> = v258.iter().take(v229.max(0) as usize).copied().collect();
            let mut v71 = 0;
            for k in 0..v191.len() {
                let next = if k + 1 < v191.len() { v191[k + 1] } else { c.g3096.len() as i32 };
                if next - v191[k] >= 113 {
                    v71 = v191[k];
                    break;
                }
            }
            let (v228, v227, v226) = d5::break_find(v229, &v258, p1, &p2, &v290, &v288);
            tr.rec("BreakFind", || vec![(7, sv(v228 as f64)), (8, sv(v227 as f64)), (9, sv(v226 as f64))]);
            let (v225, v224, v223, v222) = d5::big_change_find(v229, &v258, p1, &p2, &v290, &v288, &selfa, &c.g3064)?;
            tr.rec("BigChangeFind", || vec![(9, sv(v225 as f64)), (10, sv(v224 as f64)), (11, sv(v223 as f64)), (12, sv(v222 as f64))]);
            let (mut p14, mut v257, v258) = if v71 > 1 || v228 > 1 || v225 >= 2 {
                let (a, b, l) = d6::irregular_judge(v231, v229, &v258, &c.g3096, p1, v71, v228, v227, v225, v224, v223, v222, &p3, v226)?;
                tr.rec("IrregularJudge", || vec![(3, iv(&l)), (15, sv(a as f64)), (16, iv(&b))]);
                (a, b, l)
            } else {
                (0, vec![v231], v258)
            };
            if !(v264.is_empty() || v264[0] < 1) {
                let mut v112 = p14;
                if p14 >= 0 {
                    loop {
                        let mut k = v264.len();
                        while k > 0 {
                            k -= 1;
                            let v114 = v257.get(v112 as usize).copied().unwrap_or(0);
                            let v115 = v264[k];
                            if v114 == v115 {
                                v264.remove(k);
                            } else if v114 + 4 == v115 || v114 - 4 == v115 {
                                if let Some(x) = v257.get_mut(v112 as usize) {
                                    *x = 0;
                                }
                                p14 -= 1;
                            }
                        }
                        let mut again = false;
                        while v112 >= 1 {
                            v112 -= 1;
                            if !v264.is_empty() {
                                again = true;
                                break;
                            }
                        }
                        if !again {
                            break;
                        }
                    }
                }
                v257.extend_from_slice(&v264);
                v257.sort_by(|a, b| b.cmp(a));
                v257.retain(|x| *x != 0);
                v257.sort();
                p14 = v257.len() as i32 - 1;
            }
            let v256 = d6::verse_hook_judge_all(p14, &v257, p1, &c.g3096, &p3, &v240, &v239, &c.g3064, &p2, &v290, &selfa, &v288)?;
            tr.rec("VerseHookJudge_All", || vec![(14, iv(&v256))]);
            let v255 = d6::candidate_adjust(&v256, &v288, &p2);
            tr.rec("CandidateAdjust", || vec![(5, iv(&v255))]);
            let dur = inp.total as f64 / inp.sr;
            let mut v200r = if dur * 0.052000000000000005 >= v255.len() as f64 {
                v255.clone()
            } else {
                let r = &c.g3448[..c.n3448.min(c.g3448.len())];
                let (v254, v184b, v188b, v185) = d6::rhythm_pattern_reject(&v255, r);
                tr.rec("RhythmPatternReject", || vec![(4, iv(&v254)), (5, sv(v184b)), (6, fv(&v188b)), (7, uv(&v185))]);
                v254
            };
            v200r.insert(0, inp.b.f600);
            let v190 = v200r.clone();
            v200 = d7::verse_hook_judge23(&vh, &v259, v231, p1, p18_0)?;
            tr.rec("VerseHookJudge23", || vec![(14, iv(&v200))]);
            let v189 = v200.clone();
            v200 = d8::verse_hook_add_genre1(&vh[1].v, &vh[1].fold, &vh[1].sign, &c.g3224, &mp, &v286, &v190, &v189, p1)?;
            tr.rec("VerseHookAdd_genre1", || vec![(10, iv(&v200))]);
            for x in v200.iter_mut() {
                if *x > v181 + 3 {
                    break;
                }
                *x = 0;
            }
            let k0 = v200.iter().take_while(|x| **x == 0).count();
            v200.drain(..k0);
            v200.insert(0, v181);
            for &sp in v262.iter().rev() {
                let mut j = v200.len();
                while j >= 1 {
                    let v132 = v200[j - 1];
                    if sp == v132 {
                        break;
                    }
                    j -= 1;
                    if sp > v132 {
                        v200.insert(j + 1, sp);
                        break;
                    }
                }
            }
            for &sp in v262.iter().rev() {
                if v200.len() >= 2 {
                    let mut j = v200.len();
                    while j >= 2 {
                        let v160 = v200[j - 1];
                        if !(sp - 8 > v160 || sp + 8 < v160) && sp != v160 {
                            v200.remove(j - 1);
                        }
                        j -= 1;
                    }
                }
            }
            if v200.len() > 1 && v200[0] == v200[1] {
                v200.remove(0);
            }
            let _ = (v226, v227, v258, v255);
        }
        if v230 == 0 {
            v200 = vec![v181];
        }
    } else {
        return None; // the Low-mood / type-2 structure path is not ported yet
    }
    if mood == 1 {
        if cancelled() { return None; }
        let (mut v270b, mut v269b, v268b) = d4::block_average(&v200, &selfa, &c.g3064, p1, &c.g3128)?;
        tr.rec("BlockAverage", || vec![(7, fv(&v270b)), (8, fv(&v269b)), (9, fv(&v268b))]);
        let (v253, v220) = d9::song_structure_judge(&v200, &mut v270b, &mut v269b)?;
        tr.rec("SongStructureJudge", || vec![(2, fv(&v270b)), (3, fv(&v269b)), (6, iv(&v253)), (7, sv(v220 as f64))]);
        let mut v253 = v253;
        let mut v199 = vec![0i32; v200.len()];
        let (v219, v218) = d10::intro_feature_ana(&v200, &mut v253, &c.g3064, &c.g3128, &v268b, &mut v199, &v286, v237);
        tr.rec("IntroFeatureAna", || vec![(2, iv(&v253)), (6, iv(&v199)), (9, sv(v219 as f64)), (10, sv(v218 as f64))]);
        tr.rec("VerseFeatureAna", || vec![(5, sv(0.0)), (6, sv(0.0))]);
        let v252 = v253.clone();
        let (_v249b, v267b) = d4::up_simpleloop(&v200, &mut v253, &c.g3288, &c.g3320, &v269b)?;
        tr.rec("UpSimpleloopAna", || vec![(2, iv(&v253)), (8, fv(&v267b))]);
        let (k, v194, v197, v196, v195) = d4::up4_8_16(&v200, &v253, rh2)?;
        v253 = k;
        tr.rec("Up4_8_16Ana", || vec![(2, iv(&v253)), (5, iv(&v194)), (6, iv(&v197)), (7, iv(&v196)), (8, iv(&v195))]);
        d11::up_simpleloop_b(v200.len(), &mut v253, &v252, &v267b);
        tr.rec("UpSimpleloopAna_b", || vec![(2, iv(&v253))]);
        let (v248, v247) = d11::up_feature_ana(&v200, &mut v253, &c.g3064, &v268b, &mut v199, &v286, v237);
        tr.rec("UpFeatureAna", || vec![(2, iv(&v253)), (5, iv(&v199)), (8, iv(&v248)), (9, iv(&v247))]);
        let (v246, v245) = d11::hook_feature_ana(&v200, &mut v253, &c.g3064, &c.g3128, &v268b, &mut v199, &v286, v237);
        tr.rec("HookFeatureAna", || vec![(2, iv(&v253)), (6, iv(&v199)), (9, iv(&v246)), (10, iv(&v245))]);
        let (k, v198) = d12::outro_feature_ana(&v200, &v253, &c.g3064, rh2, &c.g3128, p1);
        v253 = k;
        tr.rec("OutroFeatureAna", || vec![(2, iv(&v253)), (8, iv(&v198))]);
        let v193 = d12::fill_adjust_ana(&v200, &mut v253, &mut v199, v200.len(), &c.g3064, &c.g3000);
        tr.rec("FillAdjustAna", || vec![(2, iv(&v253)), (3, iv(&v199)), (7, iv(&v193))]);
        d13::feature_last_adjust(&v200, &mut v253, &mut v199, &v198, 1, &v269b);
        tr.rec("FeatureLastAdjust", || vec![(2, iv(&v253)), (3, iv(&v199))]);
        let n = v200.len();
        let mut rows: Vec<[i32; 11]> = (0..n).map(|k| {
            let mut r = [0i32; 11];
            r[0] = v200[k];
            r[10] = v199.get(k).copied().unwrap_or(0);
            r
        }).collect();
        let mut v193 = v193;
        d13::data_format_convert(mood, &v253, &v197, &v196, &v195, &v194, &mut v193, &mut rows);
        return Some(DOut { n: n as i32, first_ms: inp.b.first_ms, mood, bar: p1, h4: (v220 != 1) as i32, h5: (inp.key < 12) as i32, end: v198.last().copied().unwrap_or(0), rows });
    }
    let _ = (&p18_0, &v262, &v264, &p2, &p3, &v240, &v289, &v288, &v287, &v239, &v286, &v237, &p5, &p10, &v184, &v188, &v187, &v183, &p16, &v208, &p15, &v182, &p6, &p7, &mp);
    let _ = (inp.sr, inp.total, inp.key);
    // Mid/Low mood (LABEL_262)
    let n = v200.len();
    if n <= 1 {
        return None; // the reference fails here unless an earlier block set v269
    }
    if cancelled() { return None; }
    let v238 = d14::abmelo(&c.g3592, &v200, p1, c.nbeats)?;
    tr.rec("ABmeloAnalyze", || vec![(5, v238.iter().flatten().map(|x| *x as f64).collect())]);
    let mut v244 = d14::hooksolo(&v290, &selfa, &v200, &v285, mood)?;
    tr.rec("HooksoloAnalyze", || vec![(6, iv(&v244))]);
    d14::tenkaimei(&v238, &mut v244);
    tr.rec("TenkaimeiAnalyze", || vec![(2, iv(&v244))]);
    let (mut v270c, mut v269c, v188c) = d4::block_average(&v200, &selfa, &c.g3064, p1, &c.g3128)?;
    tr.rec("BlockAverage", || vec![(7, fv(&v270c)), (8, fv(&v269c)), (9, fv(&v188c))]);
    let (v253c, v182c) = d9::song_structure_judge(&v200, &mut v270c, &mut v269c)?;
    tr.rec("SongStructureJudge", || vec![(2, fv(&v270c)), (3, fv(&v269c)), (6, iv(&v253c)), (7, sv(v182c as f64))]);
    let (v180lo, v203) = d14::yokuyo(&selfa, &v200);
    tr.rec("YokuyoAnalyze", || vec![(4, sv(v180lo as f64)), (5, sv(v203))]);
    let (v186, v198) = d12::outro_feature_ana(&v200, &v253c, &c.g3064, rh2, &c.g3128, p1);
    tr.rec("OutroFeatureAna", || vec![(2, iv(&v253c)), (7, iv(&v186)), (8, iv(&v198))]);
    let mut v253 = v244;
    let mut v199 = vec![0i32; n];
    d13::feature_last_adjust(&v200, &mut v253, &mut v199, &v198, mood, &v269c);
    tr.rec("FeatureLastAdjust", || vec![(2, iv(&v253)), (3, iv(&v199))]);
    let mut rows: Vec<[i32; 11]> = (0..n).map(|k| {
        let mut r = [0i32; 11];
        r[0] = v200[k];
        r
    }).collect();
    let z = vec![0i32; n];
    let mut v193 = vec![0i32; n];
    d13::data_format_convert(mood, &v253, &z, &z, &z, &z, &mut v193, &mut rows);
    Some(DOut { n: n as i32, first_ms: inp.b.first_ms, mood, bar: p1, h4: v180lo, h5: (inp.key < 12) as i32, end: v198.last().copied().unwrap_or(0), rows })
}

#[allow(dead_code)]
fn unused(_: &Mat2) {}
