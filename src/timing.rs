use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const STAGES: [&str; 13] = [
    "head_hash",
    "source_scan",
    "state_read",
    "freshness_git",
    "query_cache_read",
    "graph_read",
    "graph_rebuild",
    "merge_graphs",
    "derived_index_build",
    "rg_run",
    "match_enrichment",
    "output",
    "total",
];

pub struct TimingCollector {
    enabled: bool,
    started_at: Instant,
    stages: BTreeMap<&'static str, Duration>,
}

impl TimingCollector {
    pub fn from_env() -> Self {
        let enabled = std::env::var("TRACEGREP_TIMINGS")
            .map(|value| value == "1")
            .unwrap_or(false);
        Self::new(enabled)
    }

    pub fn disabled() -> Self {
        Self::new(false)
    }

    fn new(enabled: bool) -> Self {
        let mut stages = BTreeMap::new();
        for stage in STAGES {
            if stage != "total" {
                stages.insert(stage, Duration::ZERO);
            }
        }
        Self {
            enabled,
            started_at: Instant::now(),
            stages,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn add(&mut self, stage: &'static str, duration: Duration) {
        if !self.enabled {
            return;
        }
        *self.stages.entry(stage).or_insert(Duration::ZERO) += duration;
    }

    pub fn measure<T, F>(&mut self, stage: &'static str, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        let started_at = Instant::now();
        let result = f();
        self.add(stage, started_at.elapsed());
        result
    }

    pub fn print(&self, label: &str) {
        if !self.enabled {
            return;
        }

        let mut parts = Vec::with_capacity(STAGES.len());
        for stage in STAGES {
            let duration = if stage == "total" {
                self.started_at.elapsed()
            } else {
                *self.stages.get(stage).unwrap_or(&Duration::ZERO)
            };
            parts.push(format!("{stage}={:.3}s", duration.as_secs_f64()));
        }
        eprintln!("tracegrep timings ({label}): {}", parts.join(" "));
    }
}
