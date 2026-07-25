use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quant {
    Q3,
    Q4,
    Q5,
    Q6,
    Q8,
}

impl Quant {
    pub const ALL: [Quant; 5] = [Quant::Q3, Quant::Q4, Quant::Q5, Quant::Q6, Quant::Q8];

    /// Approximate GGUF bytes per parameter (weights only).
    pub fn bytes_per_param(self) -> f64 {
        match self {
            Quant::Q3 => 0.49,
            Quant::Q4 => 0.60,
            Quant::Q5 => 0.70,
            Quant::Q6 => 0.80,
            Quant::Q8 => 1.06,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Quant::Q3 => "Q3_K_M",
            Quant::Q4 => "Q4_K_M",
            Quant::Q5 => "Q5_K_M",
            Quant::Q6 => "Q6_K",
            Quant::Q8 => "Q8_0",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Quant::Q3 => "q3_k_m",
            Quant::Q4 => "q4_k_m",
            Quant::Q5 => "q5_k_m",
            Quant::Q6 => "q6_k",
            Quant::Q8 => "q8_0",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Machine {
    pub total_unified_gb: f64,
    pub os_reserved_gb: f64,
    pub docker_reserved_gb: f64,
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            total_unified_gb: 64.0,
            os_reserved_gb: 10.0,
            docker_reserved_gb: 8.0,
        }
    }
}

impl Machine {
    pub fn llm_budget_gb(&self) -> f64 {
        self.total_unified_gb - self.os_reserved_gb - self.docker_reserved_gb
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    Easy,
    Possible,
    Tight,
}

impl Fit {
    pub fn en(self) -> &'static str {
        match self {
            Fit::Easy => "Easy",
            Fit::Possible => "Possible",
            Fit::Tight => "Tight / Not recommended",
        }
    }
}

pub fn fit_for(required_gb: f64, available_gb: f64) -> Fit {
    if required_gb <= available_gb * 0.75 {
        Fit::Easy
    } else if required_gb <= available_gb {
        Fit::Possible
    } else {
        Fit::Tight
    }
}

pub struct WorkloadDef {
    pub key: &'static str,
    pub name_en: &'static str,
    pub reserve_gb: f64,
}

pub const WORKLOADS: [WorkloadDef; 4] = [
    WorkloadDef {
        key: "docker_only",
        name_en: "Docker only",
        reserve_gb: 0.0,
    },
    WorkloadDef {
        key: "programming",
        name_en: "Programming (mobile/web/backend + IDE + browser)",
        reserve_gb: 4.0,
    },
    WorkloadDef {
        key: "video_editing",
        name_en: "Light video editing",
        reserve_gb: 10.0,
    },
    WorkloadDef {
        key: "gaming",
        name_en: "Gaming / heavier graphics workload",
        reserve_gb: 12.0,
    },
];

pub fn recommendation_key(params_b: f64) -> &'static str {
    if params_b <= 10.0 {
        "small"
    } else if params_b <= 20.0 {
        "mid"
    } else if params_b < 30.0 {
        "large"
    } else {
        "xl"
    }
}

pub fn recommendation_en(key: &str) -> &'static str {
    match key {
        "small" => "Default to Q6_K. Drop to Q5/Q4 only if you want more speed or huge context.",
        "mid" => "Default to Q6_K for single-model quality, or Q5_K_M for more side workloads.",
        "large" => "Default to Q4_K_M. Use Q5 only when side workloads are light and you want extra quality.",
        _ => "At 30B and above, start conservative: Q4_K_M first, then test upward.",
    }
}

#[derive(Debug, Serialize)]
pub struct QuantEstimate {
    pub key: &'static str,
    pub label: &'static str,
    pub bytes_per_param: f64,
    pub weights_gb: f64,
    pub formula_total_gb: f64,
    pub runtime_total_gb: f64,
    pub base_fit: Fit,
}

pub fn estimate(params_b: f64, quant: Quant, budget_gb: f64) -> QuantEstimate {
    let weights_gb = params_b * quant.bytes_per_param();
    QuantEstimate {
        key: quant.key(),
        label: quant.label(),
        bytes_per_param: quant.bytes_per_param(),
        weights_gb,
        formula_total_gb: weights_gb * 1.1,
        runtime_total_gb: weights_gb * 1.5,
        base_fit: fit_for(weights_gb * 1.5, budget_gb),
    }
}

#[derive(Debug, Serialize)]
pub struct MachineOut {
    pub total_unified_gb: f64,
    pub os_reserved_gb: f64,
    pub docker_reserved_gb: f64,
    pub llm_budget_gb: f64,
}

#[derive(Debug, Serialize)]
pub struct QuantFit {
    pub key: &'static str,
    pub quant: &'static str,
    pub fit: Fit,
}

#[derive(Debug, Serialize)]
pub struct WorkloadFit {
    pub key: &'static str,
    pub name_en: &'static str,
    pub reserve_gb: f64,
    pub remaining_budget_gb: f64,
    pub fits: Vec<QuantFit>,
}

#[derive(Debug, Serialize)]
pub struct BestFit {
    pub key: &'static str,
    pub label: &'static str,
    pub fit: Fit,
    pub runtime_total_gb: f64,
}

/// Quant preference order, aligned with the rule-of-thumb advice: up to 20B
/// aim for Q6_K quality; above that start from Q4_K_M.
fn preference_for(rec_key: &str) -> &'static [Quant] {
    match rec_key {
        "small" | "mid" => &[Quant::Q6, Quant::Q5, Quant::Q4, Quant::Q3],
        _ => &[Quant::Q4, Quant::Q5, Quant::Q3],
    }
}

pub fn best_fit(params_b: f64, budget_gb: f64) -> Option<BestFit> {
    let prefs = preference_for(recommendation_key(params_b));
    for target in [Fit::Easy, Fit::Possible] {
        for &q in prefs {
            let runtime = params_b * q.bytes_per_param() * 1.5;
            if fit_for(runtime, budget_gb) == target {
                return Some(BestFit {
                    key: q.key(),
                    label: q.label(),
                    fit: target,
                    runtime_total_gb: runtime,
                });
            }
        }
    }
    None
}

pub const GENERATOR: &str = "Mulaim ملائم — LEAP RD&O واثب · https://leap.sa";

#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub generator: &'static str,
    pub input: String,
    pub source: &'static str,
    pub resolved_name: String,
    pub note_key: &'static str,
    pub note_en: String,
    pub params_b: f64,
    pub machine: MachineOut,
    pub recommendation_key: &'static str,
    pub recommendation_en: &'static str,
    pub best: Option<BestFit>,
    pub quants: Vec<QuantEstimate>,
    pub workloads: Vec<WorkloadFit>,
}

pub fn build_report(
    input: &str,
    res: &crate::resolve::Resolution,
    params_b: f64,
    machine: &Machine,
) -> CheckReport {
    let budget = machine.llm_budget_gb();
    let quants: Vec<QuantEstimate> = Quant::ALL
        .iter()
        .map(|&q| estimate(params_b, q, budget))
        .collect();

    let workloads = WORKLOADS
        .iter()
        .map(|wl| {
            let available = budget - wl.reserve_gb;
            WorkloadFit {
                key: wl.key,
                name_en: wl.name_en,
                reserve_gb: wl.reserve_gb,
                remaining_budget_gb: available.max(0.0),
                fits: Quant::ALL
                    .iter()
                    .map(|&q| QuantFit {
                        key: q.key(),
                        quant: q.label(),
                        fit: fit_for(params_b * q.bytes_per_param() * 1.5, available),
                    })
                    .collect(),
            }
        })
        .collect();

    let rec_key = recommendation_key(params_b);
    CheckReport {
        generator: GENERATOR,
        input: input.to_string(),
        source: res.source,
        resolved_name: res.resolved_name.clone(),
        note_key: res.note_key,
        note_en: res.note_en.clone(),
        params_b,
        machine: MachineOut {
            total_unified_gb: machine.total_unified_gb,
            os_reserved_gb: machine.os_reserved_gb,
            docker_reserved_gb: machine.docker_reserved_gb,
            llm_budget_gb: budget,
        },
        recommendation_key: rec_key,
        recommendation_en: recommendation_en(rec_key),
        best: best_fit(params_b, budget),
        quants,
        workloads,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_thresholds() {
        assert_eq!(fit_for(30.0, 40.0), Fit::Easy); // == 0.75 * avail
        assert_eq!(fit_for(30.1, 40.0), Fit::Possible);
        assert_eq!(fit_for(40.0, 40.0), Fit::Possible);
        assert_eq!(fit_for(40.1, 40.0), Fit::Tight);
        assert_eq!(fit_for(1.0, -2.0), Fit::Tight); // negative budget never fits
    }

    #[test]
    fn estimate_math() {
        let e = estimate(12.0, Quant::Q4, 46.0);
        assert!((e.weights_gb - 7.2).abs() < 1e-9);
        assert!((e.formula_total_gb - 7.92).abs() < 1e-9);
        assert!((e.runtime_total_gb - 10.8).abs() < 1e-9);
        assert_eq!(e.base_fit, Fit::Easy);
    }

    #[test]
    fn best_fit_selection() {
        // 12B, 46 GB budget: everything fits; rule of thumb caps at Q6_K.
        let b = best_fit(12.0, 46.0).unwrap();
        assert_eq!(b.label, "Q6_K");
        assert_eq!(b.fit, Fit::Easy);
        // 27B, 20 GB budget: only Q3_K_M squeezes in, as Possible.
        let b = best_fit(27.0, 20.0).unwrap();
        assert_eq!(b.label, "Q3_K_M");
        assert_eq!(b.fit, Fit::Possible);
        // 70B, 20 GB budget: nothing fits.
        assert!(best_fit(70.0, 20.0).is_none());
        // 32B, 110 GB budget: large-model advice starts at Q4_K_M.
        let b = best_fit(32.0, 110.0).unwrap();
        assert_eq!(b.label, "Q4_K_M");
        assert_eq!(b.fit, Fit::Easy);
    }

    #[test]
    fn recommendation_boundaries() {
        assert_eq!(recommendation_key(10.0), "small");
        assert_eq!(recommendation_key(10.1), "mid");
        assert_eq!(recommendation_key(20.0), "mid");
        assert_eq!(recommendation_key(29.9), "large");
        assert_eq!(recommendation_key(30.0), "xl");
    }
}
