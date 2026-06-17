//! Match simulation engine.
//!
//! Pipeline: team strength (Elo + xG) -> Dixon-Coles expected goals ->
//! bounded context adjustments -> 50k Monte-Carlo scoreline draws ->
//! full outcome distribution. Knockout draws resolve via extra time + penalties.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

/// Average goals scored per team per international match. Tournament baseline.
const BASE_GOALS: f64 = 1.35;
/// How strongly the Elo gap tilts the goal split. Small on purpose.
const ELO_GOAL_TILT: f64 = 0.0016;
/// Dixon-Coles low-score correlation. Negative => slightly more draws/0-0.
const DC_RHO: f64 = -0.13;
/// Max goals per side in the scoreline matrix.
const MAX_GOALS: usize = 10;
/// Default Monte-Carlo draw count.
pub const DEFAULT_SIMS: usize = 50_000;
/// Extra time is 30 of 90 minutes => one third of the scoring rate.
const EXTRA_TIME_FRACTION: f64 = 1.0 / 3.0;

#[derive(Clone, Debug, Deserialize)]
pub struct TeamStrength {
    pub name: String,
    pub elo: f64,
    /// Expected goals FOR per match (attacking quality).
    pub xg_for: f64,
    /// Expected goals AGAINST per match (defensive leakiness).
    pub xg_against: f64,
}

/// Bounded multipliers on a team's expected goals. 1.0 = no change.
/// Layer 2 (deterministic) and Layer 3 (Claude's news read) both write here.
/// Each factor is clamped so judgment can nudge, never invent.
#[derive(Clone, Debug, Deserialize)]
pub struct Adjustments {
    #[serde(default = "one")]
    pub attack: f64,
    #[serde(default = "one")]
    pub defense: f64,
}

fn one() -> f64 {
    1.0
}

impl Default for Adjustments {
    fn default() -> Self {
        Self { attack: 1.0, defense: 1.0 }
    }
}

impl Adjustments {
    fn clamped(&self) -> (f64, f64) {
        (self.attack.clamp(0.6, 1.4), self.defense.clamp(0.6, 1.4))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MatchSetup {
    pub home: TeamStrength,
    pub away: TeamStrength,
    /// True for a neutral venue (most World Cup games).
    #[serde(default)]
    pub neutral: bool,
    /// Elo bonus applied to `home` when it is a host nation playing at home.
    #[serde(default)]
    pub host_elo_bonus: f64,
    /// Knockout match => no draw allowed; resolve via ET + penalties.
    #[serde(default)]
    pub knockout: bool,
    #[serde(default)]
    pub home_adj: Adjustments,
    #[serde(default)]
    pub away_adj: Adjustments,
    /// Optional fixed seed for reproducible demos.
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub sims: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Scoreline {
    pub home: usize,
    pub away: usize,
    pub prob: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MatchOutcome {
    pub home: String,
    pub away: String,
    pub lambda_home: f64,
    pub lambda_away: f64,
    pub sims: usize,
    /// Regulation-time result probabilities.
    pub p_home_win: f64,
    pub p_draw: f64,
    pub p_away_win: f64,
    /// For knockouts: probability `home` advances (after ET + penalties).
    pub p_home_advance: Option<f64>,
    pub p_over_2_5: f64,
    pub p_btts: f64,
    pub expected_goals_home: f64,
    pub expected_goals_away: f64,
    pub top_scorelines: Vec<Scoreline>,
}

fn poisson_pmf(lambda: f64, k: usize) -> f64 {
    let mut p = (-lambda).exp();
    for i in 1..=k {
        p *= lambda / i as f64;
    }
    p
}

/// Dixon-Coles dependence correction for the four low-score cells.
fn dc_tau(i: usize, j: usize, lh: f64, la: f64, rho: f64) -> f64 {
    match (i, j) {
        (0, 0) => 1.0 - lh * la * rho,
        (0, 1) => 1.0 + lh * rho,
        (1, 0) => 1.0 + la * rho,
        (1, 1) => 1.0 - rho,
        _ => 1.0,
    }
}

/// Turn raw strength into Dixon-Coles expected goals for each side.
fn expected_goals(s: &MatchSetup) -> (f64, f64) {
    let avg = BASE_GOALS;

    let atk_h = s.home.xg_for / avg;
    let def_h = s.home.xg_against / avg;
    let atk_a = s.away.xg_for / avg;
    let def_a = s.away.xg_against / avg;

    let mut lh = avg * atk_h * def_a;
    let mut la = avg * atk_a * def_h;

    // Elo tilt: shift the goal split toward the stronger side.
    let home_elo = s.home.elo + if s.neutral { s.host_elo_bonus } else { s.host_elo_bonus + 60.0 };
    let elo_diff = home_elo - s.away.elo;
    let tilt = (ELO_GOAL_TILT * elo_diff).exp();
    lh *= tilt;
    la /= tilt;

    let (ha, hd) = s.home_adj.clamped();
    let (aa, ad) = s.away_adj.clamped();
    // A team's attack multiplier raises its own goals; the opponent's defense
    // multiplier (leakiness up) also raises this team's goals.
    lh *= ha * ad;
    la *= aa * hd;

    (lh.clamp(0.05, 6.0), la.clamp(0.05, 6.0))
}

/// Normalized DC-adjusted scoreline matrix over 0..=MAX_GOALS for each side.
fn scoreline_matrix(lh: f64, la: f64) -> Vec<Vec<f64>> {
    let n = MAX_GOALS + 1;
    let mut m = vec![vec![0.0_f64; n]; n];
    let mut total = 0.0;
    for i in 0..n {
        for j in 0..n {
            let p = poisson_pmf(lh, i) * poisson_pmf(la, j) * dc_tau(i, j, lh, la, DC_RHO);
            m[i][j] = p.max(0.0);
            total += m[i][j];
        }
    }
    for row in m.iter_mut() {
        for c in row.iter_mut() {
            *c /= total;
        }
    }
    m
}

/// Sample one (home, away) scoreline from the flattened CDF.
fn sample_scoreline(cdf: &[(usize, usize, f64)], rng: &mut StdRng) -> (usize, usize) {
    let r: f64 = rng.r#gen();
    for &(i, j, c) in cdf {
        if r <= c {
            return (i, j);
        }
    }
    cdf.last().map(|&(i, j, _)| (i, j)).unwrap_or((0, 0))
}

/// Penalty-shootout win probability for the home side, tilted slightly by Elo.
fn shootout_home_prob(s: &MatchSetup) -> f64 {
    let diff = s.home.elo - s.away.elo;
    (0.5 + diff * 0.00035).clamp(0.35, 0.65)
}

pub fn simulate(s: &MatchSetup) -> MatchOutcome {
    let (lh, la) = expected_goals(s);
    let matrix = scoreline_matrix(lh, la);
    let n = MAX_GOALS + 1;

    let mut cdf: Vec<(usize, usize, f64)> = Vec::with_capacity(n * n);
    let mut acc = 0.0;
    for i in 0..n {
        for j in 0..n {
            acc += matrix[i][j];
            cdf.push((i, j, acc));
        }
    }

    let sims = s.sims.unwrap_or(DEFAULT_SIMS).clamp(1_000, 500_000);
    let mut rng = match s.seed {
        Some(seed) => StdRng::seed_from_u64(seed),
        None => StdRng::seed_from_u64(0x60A1_D16E_2026_0609),
    };

    let (mut hw, mut dr, mut aw) = (0u64, 0u64, 0u64);
    let (mut over, mut btts) = (0u64, 0u64);
    let mut adv_home = 0u64;
    let (mut gh, mut ga) = (0u64, 0u64);
    let mut score_counts = vec![vec![0u64; n]; n];

    let et_h = lh * EXTRA_TIME_FRACTION;
    let et_a = la * EXTRA_TIME_FRACTION;
    let pen_home = shootout_home_prob(s);

    for _ in 0..sims {
        let (i, j) = sample_scoreline(&cdf, &mut rng);
        score_counts[i][j] += 1;
        gh += i as u64;
        ga += j as u64;
        if i + j > 2 {
            over += 1;
        }
        if i > 0 && j > 0 {
            btts += 1;
        }

        if i > j {
            hw += 1;
        } else if j > i {
            aw += 1;
        } else {
            dr += 1;
        }

        if s.knockout {
            let (mut a, mut b) = (i, j);
            if a == b {
                let ei = poisson_sample(et_h, &mut rng);
                let ej = poisson_sample(et_a, &mut rng);
                a += ei;
                b += ej;
            }
            let home_through = if a > b {
                true
            } else if b > a {
                false
            } else {
                rng.r#gen::<f64>() < pen_home
            };
            if home_through {
                adv_home += 1;
            }
        }
    }

    let f = sims as f64;
    let mut top: Vec<Scoreline> = Vec::new();
    for i in 0..n {
        for j in 0..n {
            if score_counts[i][j] > 0 {
                top.push(Scoreline { home: i, away: j, prob: score_counts[i][j] as f64 / f });
            }
        }
    }
    top.sort_by(|a, b| b.prob.partial_cmp(&a.prob).unwrap());
    top.truncate(6);

    MatchOutcome {
        home: s.home.name.clone(),
        away: s.away.name.clone(),
        lambda_home: round2(lh),
        lambda_away: round2(la),
        sims,
        p_home_win: round4(hw as f64 / f),
        p_draw: round4(dr as f64 / f),
        p_away_win: round4(aw as f64 / f),
        p_home_advance: if s.knockout { Some(round4(adv_home as f64 / f)) } else { None },
        p_over_2_5: round4(over as f64 / f),
        p_btts: round4(btts as f64 / f),
        expected_goals_home: round2(gh as f64 / f),
        expected_goals_away: round2(ga as f64 / f),
        top_scorelines: top,
    }
}

/// Single-elimination tournament Monte-Carlo.
///
/// Two stages: precompute P(i beats j) for every pair via a light knockout sim,
/// then roll the bracket forward `rollouts` times sampling each tie from that
/// matrix. `teams` must be in seeded bracket order and a power of two.
pub fn simulate_tournament(
    teams: &[TeamStrength],
    rollouts: usize,
    seed: u64,
) -> Result<Vec<(String, f64)>, String> {
    let t = teams.len();
    if t < 2 || (t & (t - 1)) != 0 {
        return Err(format!(
            "[goal-digger] tournament needs a power-of-two team count, got {t}"
        ));
    }

    // Pairwise advance probabilities (i as nominal home, neutral venue).
    let mut beats = vec![vec![0.0_f64; t]; t];
    for i in 0..t {
        for j in (i + 1)..t {
            let setup = MatchSetup {
                home: teams[i].clone(),
                away: teams[j].clone(),
                neutral: true,
                host_elo_bonus: 0.0,
                knockout: true,
                home_adj: Adjustments::default(),
                away_adj: Adjustments::default(),
                seed: Some(seed ^ ((i as u64) << 20) ^ (j as u64)),
                sims: Some(4_000),
            };
            let p = simulate(&setup).p_home_advance.unwrap_or(0.5);
            beats[i][j] = p;
            beats[j][i] = 1.0 - p;
        }
    }

    let rollouts = rollouts.clamp(1_000, 200_000);
    let mut rng = StdRng::seed_from_u64(seed ^ 0xC0FFEE);
    let mut titles = vec![0u64; t];

    for _ in 0..rollouts {
        let mut alive: Vec<usize> = (0..t).collect();
        while alive.len() > 1 {
            let mut next = Vec::with_capacity(alive.len() / 2);
            for pair in alive.chunks(2) {
                let (a, b) = (pair[0], pair[1]);
                let winner = if rng.r#gen::<f64>() < beats[a][b] { a } else { b };
                next.push(winner);
            }
            alive = next;
        }
        titles[alive[0]] += 1;
    }

    let f = rollouts as f64;
    let mut out: Vec<(String, f64)> = teams
        .iter()
        .enumerate()
        .map(|(i, tm)| (tm.name.clone(), round4(titles[i] as f64 / f)))
        .collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    Ok(out)
}

/// Knuth Poisson sampler for the extra-time goal draws.
fn poisson_sample(lambda: f64, rng: &mut StdRng) -> usize {
    let l = (-lambda).exp();
    let mut k = 0usize;
    let mut p = 1.0;
    loop {
        p *= rng.r#gen::<f64>();
        if p <= l {
            return k;
        }
        k += 1;
        if k > 20 {
            return k;
        }
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}
