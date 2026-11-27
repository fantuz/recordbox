//! Stage D of the phrase analyser port, part 8: extra boundaries for the High-mood structure
//! (`VerseHookAdd_genre1`, `ShortBreakDetection`, `SnareFillinDet`).

use crate::phrase_rb_d1::findpeaks;
use crate::phrase_rb_dsp::{max_val, sum8};

fn at(v: &[f64], i: i64) -> f64 {
    if i < 0 {
        0.0
    } else {
        v.get(i as usize).copied().unwrap_or(0.0)
    }
}
fn cmod4(x: i32) -> i32 {
    x % 4
}
/// Sort ascending and drop the leading zeros (`sort_val_and_delete0(true)`).
pub fn sort_del0(v: &mut Vec<i32>) {
    v.sort();
    let k = v.iter().take_while(|x| **x == 0).count();
    v.drain(..k);
}
fn sort_del0f(v: &mut Vec<f64>) -> Vec<f64> {
    // returns the buffer as the reference leaves it: kept values, then the stale tail
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    let k = v.iter().take_while(|x| **x == 0.0).count();
    let mut buf: Vec<f64> = v[k..].to_vec();
    buf.extend_from_slice(&v[n - k..]);
    v.drain(..k);
    buf
}

/// `ShortBreakDetection` → bar numbers of short breaks (one-beat drops in the 3960 series).
pub fn short_break_detection(s3960: &[f64], s3832: &[f64], p2: i32, p3: i32) -> Vec<i32> {
    let mut v26 = s3960.to_vec();
    let mx = max_val(&v26);
    let f = if mx == 0.0 { 333.33333333333331 } else { 1.0 / mx };
    for x in v26.iter_mut() {
        *x *= f;
    }
    let p2u = p2.max(0) as usize;
    v26.resize(p2u, 0.0);
    let mut v27 = s3832.to_vec();
    v27.resize(p2u, 0.0);
    let mut v28 = vec![0f64; p2u];
    let mut v29 = vec![0f64; p2u];
    let mut v30 = vec![0f64; p2u];
    let mut v31 = vec![0f64; p2u];
    let (v47, v48);
    let v49 = p2 - 4;
    if p2 < 6 {
        v47 = 0.003;
        v48 = 0.003;
    } else {
        let (mut v32, mut v33) = (0, 0);
        for k in 1..=(p2 - 5) as usize {
            if v26[k] == 0.0 {
                v26[k] = 0.003;
            }
            if v26[k - 1] == 0.0 {
                v26[k - 1] = 0.003;
            }
            let r = v26[k] / v26[k - 1];
            v28[k] = r;
            v30[k] = r;
            if r >= 1.0 {
                v30[k] = 0.0;
                v32 += 1;
            }
            if v27[k] == 0.0 {
                v27[k] = 0.003;
            }
            if v27[k - 1] == 0.0 {
                v27[k - 1] = 0.003;
            }
            let r = v27[k] / v27[k - 1];
            v29[k] = r;
            v31[k] = r;
            if r >= 1.0 {
                v31[k] = 0.0;
                v33 += 1;
            }
        }
        v47 = if v32 != 0 { sum8(&v30) / v32 as f64 } else { 0.003 };
        v48 = if v33 != 0 { sum8(&v31) / v33 as f64 } else { 0.003 };
    }
    let mut v91 = v28.clone();
    let v50 = sort_del0f(&mut v91);
    let mut v51 = v47 * 0.65000000000000002;
    if p2 >= 1 {
        let mut v52 = 0usize;
        while v52 < p2u {
            let v53 = at(&v50, v52 as i64);
            if v53 > v51 || v53 <= 0.0 {
                break;
            }
            v52 += 1;
        }
        if v52 >= 14 {
            v51 = at(&v50, 12);
        }
    }
    let p3u = p3.max(0) as usize;
    let mut v54 = vec![0i32; p3u];
    if p2 >= 21 {
        let mut v57 = 0usize;
        for v56 in 0..(v49 - 16) as usize {
            let a = v27[v56 + 15];
            let b = v27[v56 + 14];
            let v59 = v26[v56 + 15];
            if v28[v56 + 16] < v51 && v29[v56 + 16] < v48 * 0.94999999999999996 && ((v59 > 0.5 && (a > 0.2 || b > 0.2)) || (v59 > 0.2 && (a > 0.5 || b > 0.5))) {
                if v57 < v54.len() {
                    v54[v57] = v56 as i32 + 17;
                } else {
                    v54.push(v56 as i32 + 17);
                }
                v57 += 1;
            }
        }
    }
    for x in v54.iter_mut() {
        if (1..=2).contains(&cmod4(*x)) {
            *x = 0;
        }
    }
    for x in v54.iter_mut() {
        if *x != 0 {
            let i = *x as i64;
            let v65 = at(&v26, i - 1);
            let v67 = if v65 == 0.0 { 333.33333333333331 } else { 1.0 / v65 };
            if at(&v26, i) * v67 <= 2.5 && at(&v26, i + 1) * v67 <= 2.5 && at(&v26, i + 2) * v67 <= 2.5 && at(&v26, i + 3) * v67 <= 2.5 {
                *x = 0;
            }
        }
    }
    sort_del0(&mut v54);
    if v54.is_empty() {
        return vec![0; p3u];
    }
    v54.iter().map(|x| x / 4 + if x & 3 != 0 { 2 } else { 1 }).collect()
}

/// `SnareFillinDet` → bar numbers of snare fills (bars whose high-band peak sum jumps).
pub fn snare_fillin_det(hp: &[f64], p2: i32) -> Vec<i32> {
    let p2u = p2.max(0) as usize;
    let mut p3 = vec![0i32; p2u];
    let mut v11 = vec![0f64; p2u];
    let mut v12 = vec![0f64; p2u];
    let mut v13 = vec![0f64; p2u];
    let mut v14 = vec![0f64; p2u];
    let v77 = p2 - 4;
    if p2 >= 5 {
        for b in 0..v77 as usize {
            let w: Vec<f64> = (0..32).map(|i| at(hp, (32 * b + i) as i64)).collect();
            let (vals, _, _, _) = findpeaks(&w);
            v11[b] = sum8(&vals);
        }
    }
    let v20 = p2u;
    let mx = max_val(&v11);
    let f = if mx == 0.0 { 333.33333333333331 } else { 1.0 / mx };
    for x in v11.iter_mut() {
        let y = *x * f;
        *x = if y == 0.0 { 0.003 } else { y };
    }
    let (mut v48, mut v49);
    if v20 < 2 {
        v48 = 0.0025500000000000002;
        v49 = 0.0034499999999999999;
    } else {
        for k in 1..v20 {
            v12[k] = if v11[k - 1] != 0.0 { v11[k] / v11[k - 1] } else { 0.0 };
        }
        let (mut v42, mut v43) = (0, 0);
        for k in 1..v20 {
            let v47 = v12[k];
            if v47 >= 1.0 || v47 == 0.0 {
                if v47 >= 1.0 {
                    v13[k] = v47;
                    v42 += 1;
                }
            } else {
                v14[k] = v47;
                v43 += 1;
            }
        }
        v49 = if v42 != 0 { sum8(&v13) * 1.1499999999999999 / v42 as f64 } else { 0.0034499999999999999 };
        v48 = if v43 != 0 { sum8(&v14) * 0.84999999999999998 / v43 as f64 } else { 0.0025500000000000002 };
    }
    v13.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let mut v84 = v13.clone();
    let v51 = p2u / 16;
    if p2 >= 16 && v13.len() >= v51 && v13[v51 - 1] >= v49 {
        v49 = v13[v51 - 1];
    }
    let v52 = sort_del0f(&mut v84);
    if p2 < 16 || v84.len() < v51 {
        if p2 < 6 {
            return p3;
        }
    } else if at(&v52, v51 as i64 - 1) <= v48 {
        v48 = at(&v52, v51 as i64 - 1);
    }
    let mut v55 = false;
    let mut v56 = 0usize;
    let mut push = |v: i32, v56: &mut usize| {
        if *v56 < p3.len() {
            p3[*v56] = v;
        }
        *v56 += 1;
    };
    let mut v59 = 1i32;
    while v59 < v77 {
        let i = v59 as i64;
        let mut skip = v55;
        let v60 = at(&v12, i);
        if !skip && v60 <= v49 && v60 >= v48 {
            skip = true;
        }
        let v62 = at(&v12, i + 2);
        if !skip && (v62 >= 1.1499999999999999 || v62 <= 0.84999999999999998) {
            skip = true;
        }
        if skip {
            v55 = false;
        } else {
            let v64 = at(&v12, i + 1);
            let v65 = v64 * v60;
            let mut brk = v60 >= 1.0 || v64 <= v48;
            if !brk {
                let v67 = at(&v11, i - 1);
                brk = if v60 <= 0.5 { v67 <= 0.5 } else { v67 <= 0.29999999999999999 };
            }
            if brk {
                v55 = false;
            } else {
                push(if v65 < 0.80000000000000004 { v59 + 1 } else { v59 + 2 }, &mut v56);
                v55 = true;
            }
            // LABEL_97
            if !(v60 <= 1.0 || v64 >= v49) {
                let v70 = at(&v11, i);
                let go106 = if v60 >= 2.0 { !(v70 <= 0.5 && !v55) } else { v55 || v70 > 0.40000000000000002 };
                if go106 {
                    push(if v65 > 0.80000000000000004 { v59 + 1 } else { v59 + 2 }, &mut v56);
                    v55 = true;
                } else {
                    v55 = false;
                }
            }
        }
        v59 += 1;
    }
    p3
}

fn to_bar(x: i32, bar: i32) -> i32 {
    let v = (x - bar) as f64 * 0.25;
    (v + if v >= 0.0 { 0.5 } else { -0.5 }) as i32 + 1
}

/// `VerseHookAdd_genre1` → boundary beats; also returns the converted `p6`/`p7` lists.
#[allow(clippy::too_many_arguments)]
pub fn verse_hook_add_genre1(llv: &[f64], llf: &[f64], lls: &[f64], s3960: &[f64], s3832: &[f64], hp: &[f64], p6in: &[i32], p7in: &[i32], bar: i32) -> Option<Vec<i32>> {
    let mut p6s = p6in.to_vec();
    sort_del0(&mut p6s);
    let mut v164 = vec![0i32; p6in.len()];
    for (k, x) in p6s.iter().enumerate() {
        v164[k] = to_bar(*x, bar);
    }
    let k0 = v164.iter().take_while(|x| **x == 0).count();
    v164.drain(..k0);
    let p6 = v164;
    let mut p7 = p7in.to_vec();
    sort_del0(&mut p7);
    for x in p7.iter_mut() {
        *x = to_bar(*x, bar);
    }
    let k0 = p7.iter().take_while(|x| **x == 0).count();
    p7.drain(..k0);
    for v78 in 0..p7.len() {
        let mut v79 = false;
        let mut v80 = p7[v78];
        for &v83 in &p6 {
            if (v80 - v83).abs() != 1 {
                continue;
            }
            let i = v80 as i64 - 1;
            if at(llf, i) < 1.5 || at(llv, i) < 0.5 {
                v79 = true;
                p7[v78] = v83;
                v80 = v83;
            } else if v79 {
                v79 = true;
            } else if v83 - v80 == 1 {
                v79 = false;
                if at(lls, i) <= 0.5 && at(lls, v83 as i64 - 1) >= 1.8 {
                    p7[v78] = v83;
                    v80 = v83;
                }
            } else {
                v79 = false;
            }
        }
    }
    let mut v158 = s3832.to_vec();
    let _ = sort_del0f(&mut v158);
    let v73 = v158.len() as i32;
    let v74 = v73 / 4;
    let sb = short_break_detection(s3960, s3832, v73, v74);
    let mut p8a = p7;
    p8a.extend_from_slice(&sb);
    let sf = snare_fillin_det(hp, v74);
    let mut p7a = p8a.clone();
    p7a.extend_from_slice(&sf);
    let mut v76 = p7a.clone();
    v76.sort();
    let n = v76.len();
    for i in 0..n {
        for j in 0..n {
            if i != j && v76[i] == v76[j] {
                v76[j] = 0;
            }
        }
    }
    sort_del0(&mut v76);
    let mut v162 = v76;
    for x in &p6 {
        for y in v162.iter_mut() {
            if *y == *x {
                *y = 0;
            }
        }
    }
    let mut p7a = p6.clone();
    p7a.extend_from_slice(&v162);
    sort_del0(&mut p7a);
    sort_del0(&mut v162);
    let v108 = p7a.len();
    let mut v109 = vec![0u8; v108];
    let mut v161 = p7a.clone();
    if v108 >= 2 {
        if !v162.is_empty() {
            for k in 1..v108 {
                if v162.contains(&p7a[k]) {
                    v109[k] = 1;
                }
            }
        }
        let v115 = v108 - 1;
        let mut v114 = 0usize;
        for v116 in 1..v108 {
            let v121 = v116 - v114;
            let v122 = v161[v121];
            let prev = v161[v121 - 1];
            let v123 = v122 - prev;
            let v124 = cmod4(v123);
            let (mut v119, mut v120) = (0, 0);
            if v116 < v115 {
                v119 = v161.get(v121 + 1).copied().unwrap_or(0) - v122;
                v120 = cmod4(v119);
            }
            if v109[v116] == 1 && v124 != 0 && v115 != v116 {
                let v126 = if v123 == 1 || v119 == 1 {
                    2
                } else {
                    let v127 = at(lls, v122 as i64 - 1);
                    if v127 > 0.82999999999999996 {
                        2
                    } else {
                        let mut t = 2;
                        if v123 != 2 && v124 != 2 {
                            t = 0;
                        }
                        if v119 != 2 {
                            t = 0;
                        }
                        t
                    }
                };
                let v132 = v123 >= 4 && v124 == 3 && v119 >= 4 && v120 == 1;
                let v133 = at(lls, v122 as i64 - 1);
                if v132 && v133 <= 0.82999999999999996 && at(lls, v122 as i64) * v133 >= 0.90000000000000002 {
                    v161[v121] = v122 + 1;
                } else if v126 == 2 {
                    v161.remove(v121);
                    v114 += 1;
                }
            }
        }
    }
    let mut p7a = v161;
    if let Some(&l) = p7a.last() {
        if l > llv.len() as i32 - 4 {
            p7a.pop();
        }
    }
    Some(p7a.iter().map(|x| bar - 4 + 4 * x).collect())
}
