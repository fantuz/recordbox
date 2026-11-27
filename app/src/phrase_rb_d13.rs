//! Stage D of the phrase analyser port, part 13: final kind fixes (`FeatureLastAdjust`).

fn ch(k: i32) -> bool {
    k == 40 || k == 41 || k == 440 || k == 441
}

/// `FeatureLastAdjust`: last fixes of the kinds (short choruses, build-up variants, outro) and
/// of the fill beats.
pub fn feature_last_adjust(pos: &[i32], k: &mut [i32], fills: &mut [i32], v198: &[i32], p4: i32, v269: &[f64]) {
    let n = k.len();
    let mut v17 = pos.to_vec();
    v17.push(*v198.last().unwrap_or(&0));
    let g = |i: usize| v17.get(i).copied().unwrap_or(0);
    if n < 2 {
        return;
    }
    let v18 = n - 1;
    if g(2) - g(1) <= 8 {
        let mut v19 = k[1];
        if ch(v19) {
            let v21 = k[0];
            let v20 = !ch(v21);
            if n == 2 {
                if v20 {
                    v19 = if v19 & !1 == 40 { 60 } else { 61 };
                }
            } else {
                let v24 = k[2];
                if !ch(v24) && v20 {
                    let mut v25 = if v19 == 40 { 10 } else { k[1] };
                    if v19 == 41 {
                        v25 = 11;
                    }
                    if v19 == 440 {
                        v25 = 110;
                    }
                    if v19 == 441 {
                        v25 = 111;
                    }
                    if v21 == 50 {
                        v25 = 50;
                    }
                    v19 = if !(330..=333).contains(&v21) && !(30..=33).contains(&v21) { v25 } else if v19 == 40 { 30 } else { 330 };
                }
            }
        }
        k[1] = v19;
    }
    if n != 2 {
        for i in 2..n {
            let v35 = n - 1 - i;
            if g(i + 1) - g(i) > 8 {
                continue;
            }
            let mut v39 = k[i];
            if ch(v39) {
                let v40 = k[i - 1];
                let mut v41 = !ch(v40) as i32;
                if v35 != 0 && !ch(k[i + 1]) {
                    v41 += 1;
                }
                if v35 == 0 && v41 == 1 {
                    v39 = if v39 & !1 == 40 { 60 } else { 61 };
                } else if v35 != 0 && v41 == 2 {
                    let v45 = if v40 == 50 { 50 } else { k[i] };
                    v39 = if !(330..=333).contains(&v40) && !(30..=33).contains(&v40) { v45 } else if v39 == 40 { 30 } else { 330 };
                }
            }
            k[i] = v39;
        }
    }
    if p4 != 1 {
        return;
    }
    for i in 1..n {
        let v30 = n - 1 - i;
        let v32 = k[i];
        let mut v31 = k[i];
        if g(i + 1) - g(i) <= 31 && (v32 == 440 || v32 == 40) {
            let c = if v32 == 40 { 41 } else { 441 };
            if k[i - 1] == 441 || k[i - 1] == 41 {
                v31 = c;
            }
            if v30 != 0 && (k[i + 1] == 441 || k[i + 1] == 41) {
                v31 = c;
            }
        }
        k[i] = v31;
    }
    let v47 = n as i64 - 2;
    for v46 in 0..v18 {
        let v53 = g(v46 + 1) - g(v46);
        let mut v54 = v53 < 5;
        let v55 = k[v46 + 1];
        let v56 = k[v46];
        let mut go92 = v55 == 50;
        if g(v46 + 2) - g(v46 + 1) <= 4 {
            if !ch(v55) || v53 < 5 {
                go92 = true;
            } else {
                v54 = v47 == v46 as i64;
                if (v46 as i64) <= n as i64 - 3 {
                    let v57 = k[v46 + 2];
                    let v61 = if v57 == 440 { v47 == v46 as i64 } else { v57 != 441 || v47 == v46 as i64 };
                    if !(v57 == 40 || v57 == 41) {
                        v54 = v61;
                    }
                }
            }
        }
        if go92 || v55 == 60 || v54 {
            if let Some(f) = fills.get_mut(v46) {
                *f = 0;
            }
            let v50 = k[v46];
            k[v46] = match v50 {
                110 => 10,
                111 => 11,
                330 => 30,
                331 => 31,
                332 => 32,
                333 => 33,
                440 => 40,
                441 => 41,
                _ => v56,
            };
        }
    }
    if n > 3 {
        for i in 2..n - 1 {
            if g(i + 1) - g(i) <= 12 {
                let p = k[i - 1];
                if k[i] == 330 {
                    k[i] = if p == 331 || p == 31 { 331 } else { 330 };
                } else if k[i] == 30 {
                    k[i] = if p == 331 || p == 31 { 31 } else { 30 };
                }
            }
        }
    }
    if n >= 3 {
        for i in 1..n - 1 {
            if g(i + 1) - g(i) >= 69 {
                if k[i] == 31 {
                    k[i] = 30;
                }
                if k[i] == 331 {
                    k[i] = 330;
                }
            }
        }
    }
    // LABEL_167
    if n > 2 && (31..=33).contains(&k[n - 2]) {
        k[n - 2] = 30;
    }
    if n >= 5 {
        if !(1..n - 1).any(|i| ch(k[i])) {
            for v80 in 2..n - 2 {
                let v82 = k[v80 - 1];
                if ((331..=333).contains(&v82) || v82 == 31 || v82 == 32) && k[v80 + 1] == 50 {
                    k[v80] = if k[n - 1] == 61 { 41 } else { 40 };
                }
            }
        }
        for i in 1..n - 1 {
            let r = v269.get(i).copied().unwrap_or(0.0);
            if k[i] & !1 == 332 {
                if r >= 0.28000000000000003 {
                    k[i] = 330;
                }
            } else if k[i] & !1 == 32 && r >= 0.28000000000000003 {
                k[i] = 30;
            }
        }
    }
    for i in 0..v18 {
        if let Some(f) = fills.get_mut(i) {
            if *f != 0 && g(i + 1) - *f >= 5 {
                *f = g(i + 1) - 4;
            }
        }
    }
}

const LABELS: [i32; 6] = [7, 2, 3, 4, 5, 6];

/// `DataFortmatConvert`: the internal kind codes → the stored phrase columns. `rows` already
/// has the beat (column 0) and the fill beat (column 10).
#[allow(clippy::too_many_arguments)]
pub fn data_format_convert(mood: i32, kinds: &[i32], v197: &[i32], v196: &[i32], v195: &[i32], v194: &[i32], v193: &mut [i32], rows: &mut [[i32; 11]]) {
    let n = rows.len();
    let gi = |v: &[i32], i: usize| v.get(i).copied().unwrap_or(0);
    let kd = |i: usize| kinds.get(i).copied().unwrap_or(0);
    if mood == 1 {
        for i in 0..n {
            if kd(i) <= 99 {
                if let Some(x) = v193.get_mut(i) {
                    *x = 0;
                }
            }
        }
        let mut v40 = 0i32;
        for i in 0..n {
            let v46 = kd(i);
            let mut v47 = (v46 as f64 * 0.01) as i32;
            let v48 = v46 - 100 * v47;
            let v49 = (v48 as f64 * 0.10000000000000001) as i32;
            let v50 = v48 - 10 * v49;
            let v51;
            if v46 <= 99 {
                v51 = v46 - 10 * (v46 as f64 * 0.10000000000000001) as i32;
                v47 = (v46 as f64 * 0.10000000000000001) as i32;
            } else {
                v40 = v50;
                v51 = v49;
            }
            let fill = |r: &mut [i32; 11], v193: &[i32]| r[9] = (gi(v193, i) != 1) as i32;
            let r = &mut rows[i];
            match v47 {
                1 | 4 => {
                    r[1] = if v47 == 1 { 1 } else { 5 };
                    if v46 >= 101 {
                        fill(r, v193);
                        if v40 == 1 {
                            r[2] = 1;
                        }
                    } else if v51 == 1 {
                        r[2] = 1;
                    }
                }
                2 => {
                    r[1] = 4;
                    if v46 > 100 {
                        r[9] = 1;
                    }
                }
                3 => {
                    r[1] = 2;
                    let sel = if v46 >= 101 {
                        fill(r, v193);
                        v40
                    } else {
                        v51
                    };
                    match sel {
                        1 => r[8] = 1,
                        2 => {
                            r[3] = 1;
                            r[4] = 1;
                            r[5] = gi(v197, i);
                            r[6] = gi(v196, i);
                            r[7] = gi(v195, i);
                        }
                        3 => {
                            r[3] = 1;
                            r[4] = 0;
                            r[5] = gi(v194, i);
                        }
                        _ => {}
                    }
                }
                5 => r[1] = 3,
                6 => {
                    r[1] = 6;
                    if v51 == 1 {
                        r[2] = 1;
                    }
                }
                _ => {}
            }
        }
        return;
    }
    let mut seen: Vec<i32> = vec![];
    for i in 0..n {
        let v35 = kd(i);
        let k = match v35 {
            50 => Some(8),
            60 => Some(10),
            10 => Some(1),
            40 => Some(9),
            _ => None,
        };
        if let Some(k) = k {
            rows[i][1] = k;
            continue;
        }
        if v35 < 61 {
            continue;
        }
        if let Some(p) = seen.iter().position(|x| *x == v35) {
            let j = (0..n).find(|j| kd(*j) == seen[p]).unwrap_or(0);
            rows[i][1] = rows[j][1];
        } else {
            let v34 = seen.len() + 1;
            rows[i][1] = LABELS[v34 % 6];
            seen.push(v35);
        }
    }
}
