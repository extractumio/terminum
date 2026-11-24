// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-249: the interval matches the metric's denominator and scale.

use serde::Serialize;

use super::spend::MIN_RATE_SAMPLE;

const Z: f64 = 1.96;

#[derive(Serialize)]
pub(super) struct Rate {
    sample: usize,
    value: Option<f64>,
    interval_95: Option<[Option<f64>; 2]>,
    can_gate: bool,
    missing: Option<&'static str>,
}

fn rate(n: usize, value: f64, interval: [Option<f64>; 2], missing: Option<&'static str>) -> Rate {
    Rate {
        sample: n, value: (n > 0).then_some(value), interval_95: (n > 0).then_some(interval),
        can_gate: n >= MIN_RATE_SAMPLE && missing.is_none(),
        missing: missing.or((n == 0).then_some("No observations in the window.")),
    }
}

pub(super) fn share(hit: usize, sample: usize, unavailable: Option<&'static str>) -> Rate {
    let n = sample.max(1) as f64;
    let p = hit as f64 / n;
    let center = p + Z * Z / (2. * n);
    let spread = Z * (p * (1. - p) / n + Z * Z / (4. * n * n)).sqrt();
    let weight = 1. + Z * Z / n;
    rate(if unavailable.is_some() { 0 } else { sample }, p,
        [Some(((center - spread) / weight).max(0.)), Some(((center + spread) / weight).min(1.))], unavailable)
}

/// A share as a percentage with its 95% interval, or None without observations.
pub(super) fn share_percent(hit: usize, sample: usize) -> Option<String> {
    let Rate { value: Some(value), interval_95: Some([Some(low), Some(high)]), .. } = share(hit, sample, None) else { return None };
    Some(format!("{:.1}%, 95% interval {:.1}–{:.1}%", value * 100., low * 100., high * 100.))
}

pub(super) fn questions(count: usize, sample: usize) -> Rate {
    // Poisson count interval (Byar approximation); multiple questions can come from one task.
    let exposure = sample.max(1) as f64;
    let limit = |n: f64, z: f64| n * (1. - 1. / (9. * n) + z / (3. * n.sqrt())).powi(3) / exposure;
    let count = count as f64;
    let low = if count == 0. { 0. } else { limit(count, -Z).max(0.) };
    rate(sample, count / exposure, [Some(low), Some(limit(count + 1., Z))], None)
}

pub(super) fn cost(costs: &[f64], runs: usize) -> Rate {
    let sample = costs.len();
    let n = sample.max(1) as f64;
    let average = costs.iter().sum::<f64>() / n;
    let costs_missing = (sample < runs).then_some("Some runs have no attributable price.");
    let interval = if sample > 1 {
        // Two-sided Student t critical values for small samples; above 30 use the
        // conservative value for 29 degrees of freedom.
        let t = [12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228,
            2.201, 2.179, 2.160, 2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086,
            2.080, 2.074, 2.069, 2.064, 2.060, 2.056, 2.052, 2.048, 2.045][(sample - 2).min(28)];
        let std_error = (costs.iter().map(|cost| (cost - average).powi(2)).sum::<f64>() / ((n - 1.) * n)).sqrt();
        [Some((average - t * std_error).max(0.)), Some(average + t * std_error)]
    } else {
        [Some(0.), None]
    };
    rate(sample, average, interval, costs_missing)
}
