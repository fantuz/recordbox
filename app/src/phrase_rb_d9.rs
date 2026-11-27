//! Stage D of the phrase analyser port, part 9: the High-mood phrase kinds
//! (`SongStructureJudge`): 10 intro, 30 build, 40 chorus, 50 down, 60 outro per block.

use crate::phrase_rb_dsp::{max_val, sum8};

/// The vectorised 8-lane sum the reference uses for `v118[1..m]`.
fn lane_sum(x: &[f64], from: usize, to: usize) -> f64 {
    let n = to.saturating_sub(from);
    let mut r = 0.0;
    let mut start = from;
    if n >= 8 {
        let n8 = n & !7;
        let mut acc = [0f64; 8];
        for c in 0..n8 / 8 {
            for (j, a) in acc.iter_mut().enumerate() {
                *a += x[from + 8 * c + j];
            }
        }
        r = (acc[6] + (acc[4] + (acc[2] + acc[0]))) + (acc[7] + (acc[5] + (acc[3] + acc[1])));
        start = from + n8;
    }
    for v in &x[start..to] {
        r += *v;
    }
    r
}

fn norm003(x: &[f64]) -> Vec<f64> {
    let mx = max_val(x);
    let f = if mx == 0.0 { 333.33333333333331 } else { 1.0 / mx };
    x.iter().map(|v| {
        let y = v * f;
        if y == 0.0 {
            0.003
        } else {
            y
        }
    }).collect()
}

/// `SongStructureJudge` → (kinds per position, p6 flag). `p1`/`p2` get 0.003 written in place
/// as the reference does.
pub fn song_structure_judge(pos: &[i32], p1: &mut [f64], p2: &mut [f64]) -> Option<(Vec<i32>, i32)> {
    let n = pos.len();
    let mut p6 = 1;
    if n == 2 {
        return Some((vec![10, 60], p6));
    }
    if n == 1 {
        return Some((vec![10], p6));
    }
    let mut p5 = vec![0i32; n];
    let v21 = p1.len();
    let v271 = norm003(p1);
    let v270 = norm003(&p2[..v21.min(p2.len())]);
    let g = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(0.0);
    let mut v269 = vec![0f64; v21];
    for k in 1..v21 {
        if p1[k - 1] == 0.0 {
            p1[k - 1] = 0.003;
        }
        if p1[k] == 0.0 {
            p1[k] = 0.003;
        }
        v269[k] = p1[k] / p1[k - 1];
    }
    // v280: alternating local extremes of the normalised p1
    let mut v280 = vec![g(&v271, 1)];
    if n >= 4 {
        let mean = sum8(&v271) / v271.len() as f64;
        let mut v74 = g(&v271, 1) < g(&v271, 2);
        let mut v73 = 0usize;
        loop {
            let mut skip = false;
            if pos[v73 + 3] - pos[v73 + 2] <= 15 && g(&v271, v73 + 2) <= mean * 0.65000000000000002 {
                skip = true;
            }
            if !skip {
                let v81 = g(&v271, v73 + 2);
                let v82 = g(&v271, v73 + 3);
                let add;
                if v74 {
                    if v81 <= v82 {
                        add = false;
                    } else {
                        v74 = false;
                        add = true;
                    }
                } else if v81 >= v82 {
                    v74 = false;
                    add = false;
                } else {
                    v74 = true;
                    add = true;
                }
                if add {
                    v280.push(v81);
                }
            }
            let v76 = v73 + 3;
            v73 += 1;
            if v76 as i64 >= n as i64 - 1 {
                break;
            }
        }
    }
    if v280.len() >= 4 {
        let cnt = (1..v280.len()).filter(|k| (v280[*k - 1] - v280[*k]).abs() >= 0.29999999999999999).count();
        if cnt > 2 {
            p6 = 0;
        }
    }
    // LABEL_79
    let v268 = v21;
    let v266 = v21;
    p5[0] = 10;
    let v108 = v268 - 1;
    p5[v268 - 1] = 60;
    let mut v110 = vec![0f64; v268];
    for k in 1..v266 {
        if p2[k - 1] == 0.0 {
            p2[k - 1] = 0.003;
        }
        v110[k] = p2[k] / p2[k - 1];
    }
    let v116 = if p1.is_empty() { 0.0 } else { sum8(p1) / p1.len() as f64 };
    let mut v118 = vec![0f64; v266];
    let v120 = if v116 == 0.0 { 333.33333333333331 } else { 1.0 / v116 };
    let mut v121 = 0.0;
    if v266 >= 3 {
        for k in 1..v108.max(1) {
            if k > 2 && v268 == 3 {
                break;
            }
            let v186 = p1[k];
            let mut r = 1.0;
            if v186 != 0.0 && g(&v270, k) >= 0.83999999999999997 {
                if k <= 2 && p1[k - 1] == 0.0 {
                    p1[k - 1] = 0.003;
                }
                if p1.get(k + 1).copied() == Some(0.0) {
                    p1[k + 1] = 0.003;
                }
                r = (v186 / g(p1, k + 1) + v186 / p1[k - 1]) * v186 * v120;
            }
            v118[k] = r;
            if k == 2 && v268 == 4 {
                break;
            }
        }
        v121 = lane_sum(&v118, 1, v108);
    }
    let v143 = v266 as i64 - 2;
    let v144 = v121 / (v266 as f64 - 2.0);
    let v146 = if v144 == 0.0 { 333.33333333333331 } else { 1.0 / v144 };
    let mut v142 = vec![0f64; v266];
    if v268 >= 3 {
        for k in 1..v108 {
            v142[k] = v118[k] * v146;
        }
        for i in 1..v108 {
            if p5[i - 1] == 40 {
                if g(&v270, i) <= 0.69999999999999996 {
                    continue;
                }
                let v163 = v269[i];
                if !(v163 > 0.69999999999999996 && v163 < 1.8999999999999999) {
                    continue;
                }
                p5[i] = 40;
            } else {
                let v166 = v110[i];
                let a = g(&v270, i);
                let b = p1[i];
                if (v166 > 4.0 && a > 0.80000000000000004 && b > 0.29999999999999999) || (v166 > 2.0 && a > 0.5 && b > 0.75) || (v166 > 1.2 && a > 0.65000000000000002 && b > 0.45000000000000001) {
                    p5[i] = 40;
                }
                if p5[i] != 0 {
                    continue;
                }
                let ok = if v166 <= 1.5 { a > 0.80000000000000004 && v142[i] > 1.25 } else { a > 0.80000000000000004 && v142[i] > 1.0 };
                if ok {
                    p5[i] = 40;
                }
            }
        }
    }
    let mut v274 = vec![0f64; v108];
    let mut v273 = vec![0f64; v108];
    if v268 >= 2 {
        for k in 0..v108 {
            if p5[k] == 40 {
                v274[k] = p2[k];
                v273[k] = p1[k];
            }
        }
    }
    let m4 = max_val(&v274);
    let m3 = max_val(&v273);
    let v174 = if m4 == 0.0 { 333.33333333333331 } else { 1.0 / m4 };
    let v175 = if m3 == 0.0 { 333.33333333333331 } else { 1.0 / m3 };
    if v266 >= 2 {
        for k in 0..v108 {
            if p5[k] == 40 {
                v274[k] *= v174;
                v273[k] *= v175;
            }
        }
        for k in 0..v108 {
            if p5[k] == 40 && ((v274[k] < 0.47999999999999998 && v118[k] < 1.1000000000000001) || (v273[k] < 0.47999999999999998 && v118[k] < 1.1000000000000001)) {
                p5[k] = 0;
            }
        }
        for k in 0..v108 {
            if p5[k] == 40 && p5[k + 1] == 40 && v269[k + 1] > 1.3999999999999999 {
                p5[k] = 0;
            }
        }
    }
    let v194 = if v266 <= 2 {
        false
    } else if p5[1] == 40 {
        true
    } else {
        (2..v108).any(|k| p5[k] == 40)
    };
    let mut v272 = vec![0f64; v108];
    if v266 >= 3 {
        v272[1..v108].copy_from_slice(&v110[1..v108]);
    }
    let mut go216 = v194;
    if !v194 {
        v272.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        while v272.last() == Some(&0.0) {
            v272.pop();
        }
        if v266 >= 3 {
            let (mut v215, mut v216) = (0usize, 0);
            for k in 1..v108 {
                if v110[k] == g(&v272, v215) {
                    if g(&v270, k) > 0.83999999999999997 {
                        p5[k] = 40;
                        if v216 > 2 {
                            break;
                        }
                        v216 += 1;
                    }
                    v215 += 1;
                }
            }
            go216 = true;
        }
    }
    if go216 && v266 >= 3 {
        for k in 1..v108 {
            if p5[k] == 0 && p5[k + 1] == 40 {
                p5[k] = 30;
            }
        }
        for k in 1..v108 {
            if p5[k] == 0 && p5[k - 1] == 40 {
                p5[k] = 50;
            }
        }
        for k in 1..v108 {
            if p5[k] == 0 && p5[k - 1] == 10 && g(&v271, k) < 0.5 && v269[k] < 1.1000000000000001 {
                p5[k] = 10;
            }
        }
        for k in 1..v108 {
            if p5[k] == 0 && ((v269[k] > 1.0 && p5[k + 1] != 60) || (pos[k + 1] - pos[k] <= 31 && p5[k - 1] == 30 && p5[k + 1] == 30)) {
                p5[k] = 30;
            }
        }
        for k in 1..v108 {
            if p5[k] == 0 && (p5[k + 1] == 60 || v269[k] < 1.0) {
                p5[k] = 50;
            }
        }
    }
    // LABEL_262
    if v143 >= 0 && p5[v143 as usize] == 0 {
        p5[v143 as usize] = 50;
        if v266 > 3 {
            let mut j = v108;
            for _ in 0..v266 - 3 {
                if p5[j] != 30 {
                    break;
                }
                p5[j] = 50;
                j -= 1;
            }
        }
    }
    if v268 > 2 {
        let v234 = (1..v108).filter(|k| p5[*k] == 40).count();
        for k in 1..v108 {
            let a = g(&v270, k);
            let b = g(&v271, k);
            let mut at303 = false;
            let v254 = p5[k];
            if v254 == 50 {
                at303 = true;
            } else if v254 == 0 {
                // 0
                let l293 = |p5: &mut Vec<i32>| {
                    let v253 = p5[k - 1];
                    if v253 != 10 {
                        let mut v = v253;
                        if p5[k + 1] == 60 {
                            v = 50;
                        }
                        if p5[k + 1] == 50 {
                            v = 50;
                        }
                        p5[k] = v;
                    } else {
                        p5[k] = 30;
                    }
                };
                if a <= 0.75 {
                    l293(&mut p5);
                } else if b <= 0.69999999999999996 || v234 >= 4 {
                    if b <= 0.69999999999999996 || v234 < 4 {
                        l293(&mut p5);
                    } else if p5[k + 1] != 60 && p5[k + 1] != 50 {
                        p5[k] = 30;
                    } else {
                        p5[k] = 50;
                        at303 = true;
                    }
                } else {
                    p5[k] = 40;
                }
            }
            if at303 {
                if p5[k - 1] != 30 || a <= 0.75 {
                    continue;
                }
                if b <= 0.69999999999999996 || v234 >= 4 {
                    if b <= 0.69999999999999996 || v234 < 4 {
                        continue;
                    }
                    p5[k] = if p5[k + 1] == 60 || p5[k + 1] == 50 { 50 } else { 30 };
                } else {
                    p5[k] = 40;
                }
            }
        }
    }
    // LABEL_321
    if v266 >= 2 {
        for k in 0..v108 {
            if p5[k] == 50 && p5[k + 1] != 60 && g(&v270, k) > 0.90000000000000002 && v110[k] > 0.90000000000000002 {
                p5[k] = 30;
            }
        }
    }
    Some((p5, p6))
}
