// dashbox/src/data/metrics.rs
use anyhow::Result;
use std::collections::HashMap;

use super::DataSource;

/// Parsed result from a single poll of the /metrics endpoint.
#[derive(Debug, Clone)]
pub enum MetricsData {
    /// Daemon is unreachable.
    Offline,
    /// Successfully parsed metrics.
    Live(LiveMetrics),
}

/// Fully parsed live metrics snapshot.
#[derive(Debug, Clone, Default)]
pub struct LiveMetrics {
    /// Value of minibox_active_containers gauge.
    pub active_containers: f64,
    /// Ops counters keyed by (op, status) → count.
    pub ops_counters: HashMap<(String, String), f64>,
    /// Duration p50/p95 keyed by op name.
    pub durations: HashMap<String, DurationSummary>,
}

/// p50 and p95 latency in seconds for a given op.
#[derive(Debug, Clone)]
pub struct DurationSummary {
    pub p50: f64,
    pub p95: f64,
}

pub struct MetricsSource {
    pub addr: String,
}

impl MetricsSource {
    pub fn new() -> Self {
        let addr =
            std::env::var("MINIBOX_METRICS_ADDR").unwrap_or_else(|_| "127.0.0.1:9090".to_string());
        Self { addr }
    }
}

impl DataSource for MetricsSource {
    type Data = MetricsData;

    fn load(&self) -> Result<MetricsData> {
        let url = format!("http://{}/metrics", self.addr);
        let body = match ureq::get(&url).call() {
            Ok(resp) => resp.into_string()?,
            Err(ureq::Error::Transport(t)) if t.kind() == ureq::ErrorKind::ConnectionFailed => {
                return Ok(MetricsData::Offline);
            }
            Err(e) => return Err(e.into()),
        };
        Ok(MetricsData::Live(parse_metrics(&body)))
    }
}

/// Parse Prometheus text exposition format into LiveMetrics.
pub fn parse_metrics(input: &str) -> LiveMetrics {
    let mut result = LiveMetrics::default();
    // bucket_data: op → vec of (le, cumulative_count)
    let mut bucket_data: HashMap<String, Vec<(f64, f64)>> = HashMap::new();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (name_and_labels, value_str) = match line.rsplit_once(' ') {
            Some(parts) => parts,
            None => continue,
        };
        let value: f64 = match value_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let (name, labels) = parse_name_and_labels(name_and_labels);

        if name == "minibox_active_containers" {
            result.active_containers = value;
        } else if name == "minibox_container_ops_total" {
            let op = labels.get("op").cloned().unwrap_or_default();
            let status = labels.get("status").cloned().unwrap_or_default();
            *result.ops_counters.entry((op, status)).or_insert(0.0) += value;
        } else if name == "minibox_container_op_duration_seconds_bucket" {
            let op = labels.get("op").cloned().unwrap_or_default();
            let le_str = labels.get("le").map(|s| s.as_str()).unwrap_or("+Inf");
            let le: f64 = if le_str == "+Inf" {
                f64::INFINITY
            } else {
                le_str.parse().unwrap_or(f64::INFINITY)
            };
            bucket_data.entry(op).or_default().push((le, value));
        }
    }

    // Derive p50/p95 from bucket data
    for (op, mut buckets) in bucket_data {
        buckets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let total = buckets.last().map(|(_, c)| *c).unwrap_or(0.0);
        if total == 0.0 {
            continue;
        }
        let p50 = interpolate_quantile(&buckets, 0.50, total);
        let p95 = interpolate_quantile(&buckets, 0.95, total);
        result.durations.insert(op, DurationSummary { p50, p95 });
    }

    result
}

/// Linear interpolation of a quantile from sorted (le, cumulative_count) pairs.
fn interpolate_quantile(buckets: &[(f64, f64)], q: f64, total: f64) -> f64 {
    let target = q * total;
    let mut prev_le = 0.0_f64;
    let mut prev_count = 0.0_f64;
    for &(le, count) in buckets {
        if count >= target {
            if count == prev_count {
                return prev_le;
            }
            // Linear interpolation within bucket
            let fraction = (target - prev_count) / (count - prev_count);
            return prev_le + fraction * (le - prev_le);
        }
        prev_le = le;
        prev_count = count;
    }
    prev_le
}

/// Split "metric_name{k=\"v\",k2=\"v2\"}" into (name, labels_map).
/// Also handles plain "metric_name" with no labels.
fn parse_name_and_labels(s: &str) -> (&str, HashMap<String, String>) {
    let mut labels = HashMap::new();
    match s.find('{') {
        None => (s, labels),
        Some(brace) => {
            let name = &s[..brace];
            let rest = &s[brace + 1..];
            let rest = rest.trim_end_matches('}');
            for pair in rest.split(',') {
                if let Some((k, v)) = pair.split_once('=') {
                    let v = v.trim_matches('"');
                    labels.insert(k.to_string(), v.to_string());
                }
            }
            (name, labels)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# HELP minibox_active_containers Number of active containers
# TYPE minibox_active_containers gauge
minibox_active_containers 3
# HELP minibox_container_ops_total Total container operations
# TYPE minibox_container_ops_total counter
minibox_container_ops_total{op="start",adapter="daemon",status="ok"} 42
minibox_container_ops_total{op="start",adapter="daemon",status="error"} 2
minibox_container_ops_total{op="stop",adapter="daemon",status="ok"} 10
# HELP minibox_container_op_duration_seconds Duration histogram
# TYPE minibox_container_op_duration_seconds histogram
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="0.001"} 0
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="0.002"} 5
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="0.004"} 21
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="0.008"} 40
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="+Inf"} 44
"#;

    #[test]
    fn test_parse_active_containers() {
        let m = parse_metrics(SAMPLE);
        assert_eq!(m.active_containers, 3.0);
    }

    #[test]
    fn test_parse_ops_counters() {
        let m = parse_metrics(SAMPLE);
        assert_eq!(
            m.ops_counters.get(&("start".to_string(), "ok".to_string())),
            Some(&42.0)
        );
        assert_eq!(
            m.ops_counters
                .get(&("start".to_string(), "error".to_string())),
            Some(&2.0)
        );
        assert_eq!(
            m.ops_counters.get(&("stop".to_string(), "ok".to_string())),
            Some(&10.0)
        );
    }

    #[test]
    fn test_parse_durations_p50_within_range() {
        let m = parse_metrics(SAMPLE);
        let d = m.durations.get("start").expect("start duration missing");
        // p50 of 44 total = 22nd obs; bucket [0.004,0.008] contains obs 22-40
        assert!(d.p50 >= 0.004 && d.p50 <= 0.008, "p50={}", d.p50);
    }

    #[test]
    fn test_parse_durations_p95_within_range() {
        let m = parse_metrics(SAMPLE);
        let d = m.durations.get("start").expect("start duration missing");
        // p95 of 44 total = 41.8th obs; bucket [0.008,+Inf] contains obs 41-44
        assert!(d.p95 >= 0.008, "p95={}", d.p95);
    }

    #[test]
    fn test_empty_input() {
        let m = parse_metrics("");
        assert_eq!(m.active_containers, 0.0);
        assert!(m.ops_counters.is_empty());
        assert!(m.durations.is_empty());
    }

    #[test]
    fn test_parse_name_no_labels() {
        let (name, labels) = parse_name_and_labels("minibox_active_containers");
        assert_eq!(name, "minibox_active_containers");
        assert!(labels.is_empty());
    }

    #[test]
    fn test_parse_name_with_labels() {
        let (name, labels) =
            parse_name_and_labels(r#"minibox_container_ops_total{op="start",status="ok"}"#);
        assert_eq!(name, "minibox_container_ops_total");
        assert_eq!(labels.get("op").map(|s| s.as_str()), Some("start"));
        assert_eq!(labels.get("status").map(|s| s.as_str()), Some("ok"));
    }
}
