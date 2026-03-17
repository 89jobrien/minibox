use serde::Serialize;

#[derive(Serialize, Default)]
struct BenchReport {
    metadata: Metadata,
    suites: Vec<SuiteResult>,
    errors: Vec<String>,
}

impl BenchReport {
    fn empty() -> Self {
        Self::default()
    }
}

#[derive(Serialize, Default)]
struct Metadata {
    timestamp: String,
    hostname: String,
    git_sha: String,
    minibox_version: String,
}

#[derive(Serialize, Default)]
struct SuiteResult {
    name: String,
    tests: Vec<TestResult>,
}

#[derive(Serialize, Default)]
struct TestResult {
    name: String,
    iterations: usize,
    durations_micros: Vec<u64>,
}

fn main() {
    println!("minibox-bench: not yet implemented");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_serializes() {
        let report = BenchReport::empty();
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"metadata\""));
    }
}
