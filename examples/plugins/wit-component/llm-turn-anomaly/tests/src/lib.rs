#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    // ── Minimal types mirroring the plugin's core data ────────────────

    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    struct MockExchange {
        call_action_id: String,
        request_action_id: String,
        response_action_id: Option<String>,
        process_id: String,
        model: Option<String>,
        started_at: u64,
        completed_at: Option<u64>,
        request_body_bytes: u64,
        request_complete: bool,
        response_body_bytes: Option<u64>,
        response_complete: bool,
    }

    #[derive(Eq, Ord, PartialEq, PartialOrd)]
    struct ExchangeGroup {
        process_id: String,
        model: Option<String>,
    }

    // ── Copied pure functions from lib.rs ─────────────────────────────

    fn median(values: &[u64]) -> u64 {
        let mut ordered = values.to_vec();
        ordered.sort_unstable();
        let midpoint = ordered.len() / 2;
        if ordered.len() % 2 == 1 {
            ordered[midpoint]
        } else {
            ((u128::from(ordered[midpoint - 1]) + u128::from(ordered[midpoint])) / 2) as u64
        }
    }

    fn similar_requests(a: &MockExchange, b: &MockExchange, tolerance_per_mille: u64) -> bool {
        if a.process_id != b.process_id {
            return false;
        }
        if a.model != b.model {
            return false;
        }
        let a_bytes = a.request_body_bytes;
        let b_bytes = b.request_body_bytes;
        if a_bytes == b_bytes {
            return true;
        }
        if tolerance_per_mille == 0 {
            return false;
        }
        let (larger, smaller) = if a_bytes >= b_bytes {
            (a_bytes, b_bytes)
        } else {
            (b_bytes, a_bytes)
        };
        if smaller == 0 {
            return larger == 0;
        }
        let diff = larger - smaller;
        diff * 1000 <= larger * tolerance_per_mille
    }

    fn group_exchanges(
        exchanges: &[MockExchange],
    ) -> BTreeMap<ExchangeGroup, Vec<&MockExchange>> {
        let mut groups: BTreeMap<ExchangeGroup, Vec<&MockExchange>> = BTreeMap::new();
        for exchange in exchanges {
            let group = ExchangeGroup {
                process_id: exchange.process_id.clone(),
                model: exchange.model.clone(),
            };
            groups.entry(group).or_default().push(exchange);
        }
        groups
    }

    // ── Rule 1: High-frequency detection ─────────────────────────────

    fn detect_high_frequency(
        exchanges: &[MockExchange],
        window_size_ms: u64,
        threshold: usize,
        min_exchanges: usize,
    ) -> Vec<(String, Option<String>, usize, u64, u64)> {
        let groups = group_exchanges(exchanges);
        let mut findings = Vec::new();

        for (group, group_exchanges) in &groups {
            if group_exchanges.len() < min_exchanges {
                continue;
            }
            let mut sorted = group_exchanges.clone();
            sorted.sort_by_key(|e| e.started_at);
            let mut window_start = 0usize;
            for window_end in 0..sorted.len() {
                while sorted[window_end].started_at - sorted[window_start].started_at
                    > window_size_ms
                {
                    window_start += 1;
                }
                let count = window_end - window_start + 1;
                if count >= threshold {
                    findings.push((
                        group.process_id.clone(),
                        group.model.clone(),
                        count,
                        sorted[window_start].started_at,
                        sorted[window_end].started_at,
                    ));
                }
            }
        }
        findings
    }

    #[test]
    fn high_frequency_detects_burst() {
        let mut exchanges = Vec::new();
        for i in 0..40 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000, // every 1s
                completed_at: Some(i * 1000 + 500),
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        let findings = detect_high_frequency(&exchanges, 30_000, 30, 10);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].0, "p1");
        assert!(findings[0].2 >= 30);
    }

    #[test]
    fn high_frequency_no_burst_under_threshold() {
        let mut exchanges = Vec::new();
        for i in 0..20 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 5000, // every 5s, spread out
                completed_at: Some(i * 5000 + 500),
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        let findings = detect_high_frequency(&exchanges, 30_000, 30, 10);
        assert!(findings.is_empty());
    }

    #[test]
    fn high_frequency_respects_min_exchanges() {
        let mut exchanges = Vec::new();
        for i in 0..5 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 100,
                completed_at: Some(i * 100 + 50),
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        // min_exchanges=10 but only 5 exchanges
        let findings = detect_high_frequency(&exchanges, 30_000, 3, 10);
        assert!(findings.is_empty());
    }

    // ── Rule 2: Consecutive retry detection ──────────────────────────

    fn detect_consecutive_retry(
        exchanges: &[MockExchange],
        consecutive_count: usize,
        min_request_bytes: usize,
    ) -> Vec<(String, Option<String>, usize, String, String, u64, u64)> {
        let groups = group_exchanges(exchanges);
        let mut findings = Vec::new();

        for (group, group_exchanges) in &groups {
            let mut sorted = group_exchanges.clone();
            sorted.sort_by_key(|e| e.started_at);
            let mut consecutive = 0usize;
            let mut first_idx = 0usize;

            for (i, exchange) in sorted.iter().enumerate() {
                let is_error = !exchange.response_complete;
                let size_ok = exchange.request_body_bytes >= min_request_bytes as u64;
                if is_error && size_ok {
                    if consecutive == 0 {
                        first_idx = i;
                    }
                    consecutive += 1;
                } else {
                    if consecutive >= consecutive_count {
                        findings.push((
                            group.process_id.clone(),
                            group.model.clone(),
                            consecutive,
                            sorted[first_idx].request_action_id.clone(),
                            sorted[i - 1].request_action_id.clone(),
                            sorted[first_idx].started_at,
                            sorted[i - 1].started_at,
                        ));
                    }
                    consecutive = 0;
                }
            }
            if consecutive >= consecutive_count {
                let last = sorted.len() - 1;
                findings.push((
                    group.process_id.clone(),
                    group.model.clone(),
                    consecutive,
                    sorted[first_idx].request_action_id.clone(),
                    sorted[last].request_action_id.clone(),
                    sorted[first_idx].started_at,
                    sorted[last].started_at,
                ));
            }
        }
        findings
    }

    #[test]
    fn consecutive_retry_detects_run() {
        let mut exchanges = Vec::new();
        // 2 successful, then 4 failures, then 1 success
        for i in 0..7 {
            let is_error = i >= 2 && i <= 5;
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: if is_error {
                    None
                } else {
                    Some(format!("res-{i}"))
                },
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: if is_error {
                    None
                } else {
                    Some(i * 1000 + 500)
                },
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: if is_error { None } else { Some(512) },
                response_complete: !is_error,
            });
        }
        let findings = detect_consecutive_retry(&exchanges, 3, 0);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].2, 4); // 4 consecutive retries
        assert_eq!(findings[0].3, "req-2");
        assert_eq!(findings[0].4, "req-5");
    }

    #[test]
    fn consecutive_retry_no_match_under_threshold() {
        let mut exchanges = Vec::new();
        for i in 0..5 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: None,
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: None,
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            });
        }
        // consecutive_count=5, exactly 5 failures
        let findings = detect_consecutive_retry(&exchanges, 5, 0);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].2, 5);

        // consecutive_count=6, not enough
        let findings = detect_consecutive_retry(&exchanges, 6, 0);
        assert!(findings.is_empty());
    }

    #[test]
    fn consecutive_retry_respects_min_request_bytes() {
        let mut exchanges = Vec::new();
        for i in 0..4 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: None,
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: None,
                request_body_bytes: 100, // small
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            });
        }
        // min_request_bytes=500, all requests are too small
        let findings = detect_consecutive_retry(&exchanges, 3, 500);
        assert!(findings.is_empty());
    }

    #[test]
    fn consecutive_retry_small_request_breaks_run() {
        let mut exchanges = Vec::new();
        // 3 failures with large bodies, 1 failure with a tiny body, then 3 more large failures
        let sizes: Vec<u64> = vec![1024, 1024, 1024, 64, 1024, 1024, 1024];
        for (i, &size) in sizes.iter().enumerate() {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: None,
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i as u64 * 1000,
                completed_at: None,
                request_body_bytes: size,
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            });
        }
        // min_request_bytes=500: the 64-byte request is not counted AND breaks the
        // run into two runs of 3, instead of one run of 7.
        let findings = detect_consecutive_retry(&exchanges, 3, 500);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].2, 3); // retry_length
        assert_eq!(findings[0].3, "req-0"); // first_action_id
        assert_eq!(findings[0].4, "req-2"); // last_action_id
        assert_eq!(findings[1].2, 3);
        assert_eq!(findings[1].3, "req-4");
        assert_eq!(findings[1].4, "req-6");

        // threshold 4: neither split run is long enough → no finding
        let findings = detect_consecutive_retry(&exchanges, 4, 500);
        assert!(findings.is_empty());

        // Control: count the tiny request too → one uninterrupted run of 7
        let findings = detect_consecutive_retry(&exchanges, 3, 0);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].2, 7);
        assert_eq!(findings[0].3, "req-0");
        assert_eq!(findings[0].4, "req-6");
    }

    // ── Rule 3: Repeated similar requests ────────────────────────────

    fn detect_repeated_similar(
        exchanges: &[MockExchange],
        similarity_window: usize,
        min_repeat_count: usize,
        tolerance_per_mille: u64,
    ) -> Vec<(String, Option<String>, usize, String, u64, u64, u64)> {
        let groups = group_exchanges(exchanges);
        let mut findings = Vec::new();

        for (group, group_exchanges) in &groups {
            let mut sorted = group_exchanges.clone();
            sorted.sort_by_key(|e| e.started_at);

            if sorted.len() < similarity_window {
                continue;
            }

            let mut i = 0;
            while i + similarity_window <= sorted.len() {
                let slice = &sorted[i..i + similarity_window];
                let mut representative_idx = 0usize;
                let mut max_repeat = 1usize;

                let mut j = 0;
                while j < slice.len() {
                    let mut run_count = 1usize;
                    let mut k = j + 1;
                    while k < slice.len() {
                        if similar_requests(slice[j], slice[k], tolerance_per_mille) {
                            run_count += 1;
                            k += 1;
                        } else {
                            break;
                        }
                    }
                    if run_count > max_repeat {
                        max_repeat = run_count;
                        representative_idx = j;
                    }
                    j = k;
                }

                if max_repeat >= min_repeat_count {
                    let rep = slice[representative_idx];
                    let last_in_run = slice[representative_idx + max_repeat - 1];
                    findings.push((
                        group.process_id.clone(),
                        group.model.clone(),
                        max_repeat,
                        rep.request_action_id.clone(),
                        rep.request_body_bytes,
                        rep.started_at,
                        last_in_run.started_at,
                    ));
                    i += max_repeat;
                } else {
                    i += 1;
                }
            }
        }
        findings
    }

    #[test]
    fn repeated_similar_detects_duplicates() {
        let mut exchanges = Vec::new();
        // 5 identical requests (same bytes), then 5 different ones
        for i in 0..10 {
            let bytes = if i < 5 { 1024 } else { 2048 + i * 100 };
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: Some(i * 1000 + 500),
                request_body_bytes: bytes,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        let findings = detect_repeated_similar(&exchanges, 10, 3, 50);
        assert!(!findings.is_empty());
        assert!(findings[0].2 >= 3);
    }

    #[test]
    fn repeated_similar_no_match_when_all_different() {
        let mut exchanges = Vec::new();
        for i in 0..10 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: Some(i * 1000 + 500),
                request_body_bytes: 1000 + i * 500, // all very different
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        let findings = detect_repeated_similar(&exchanges, 5, 3, 50);
        assert!(findings.is_empty());
    }

    #[test]
    fn repeated_similar_verifies_representative_action_id() {
        let mut exchanges = Vec::new();
        // 4 identical requests, then 4 clearly different ones
        for i in 0..8u64 {
            let bytes = if i < 4 { 2048 } else { 4096 + i * 1000 };
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: Some(i * 1000 + 500),
                request_body_bytes: bytes,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        let findings = detect_repeated_similar(&exchanges, 8, 3, 50);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].2, 4); // repeat_count
        assert_eq!(findings[0].3, "req-0"); // representative_action_id
        assert_eq!(findings[0].4, 2048); // representative_request_bytes
    }

    #[test]
    fn repeated_similar_does_not_mix_processes() {
        // Interleave identical requests from two processes. Each process alone
        // has only 4 identical requests, so mixing them would be wrong.
        let mut interleaved = Vec::new();
        let processes = ["p1", "p2", "p1", "p2", "p1", "p2", "p1", "p2"];
        for (i, process) in processes.iter().enumerate() {
            interleaved.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: (*process).into(),
                model: Some("gpt-4".into()),
                started_at: i as u64 * 1000,
                completed_at: Some(i as u64 * 1000 + 500),
                request_body_bytes: 2048,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        // min_repeat_count=5: 4 per process must NOT combine into a cross-process run
        let findings = detect_repeated_similar(&interleaved, 8, 5, 50);
        assert!(findings.is_empty());

        // Positive control: identical blocks per process → two isolated findings
        let mut blocked = Vec::new();
        for i in 0..8u64 {
            let process = if i < 4 { "p1" } else { "p2" };
            blocked.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: process.into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: Some(i * 1000 + 500),
                request_body_bytes: 2048,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        let findings = detect_repeated_similar(&blocked, 4, 3, 50);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].0, "p1");
        assert_eq!(findings[1].0, "p2");
        assert_eq!(findings[0].2, 4);
        assert_eq!(findings[1].2, 4);
        assert_eq!(findings[0].3, "req-0");
        assert_eq!(findings[1].3, "req-4");
    }

    // ── Rule 4: Error ratio detection ────────────────────────────────

    fn detect_error_ratio(
        exchanges: &[MockExchange],
        minimum_exchanges: usize,
        error_ratio_per_mille: u64,
    ) -> Vec<(String, Option<String>, usize, usize, u64)> {
        let groups = group_exchanges(exchanges);
        let mut findings = Vec::new();

        for (group, group_exchanges) in &groups {
            let total = group_exchanges.len();
            if total < minimum_exchanges {
                continue;
            }
            let error_count = group_exchanges.iter().filter(|e| !e.response_complete).count();
            if error_count == 0 {
                continue;
            }
            let actual_ratio = (error_count as u64) * 1000 / (total as u64);
            if actual_ratio >= error_ratio_per_mille {
                findings.push((
                    group.process_id.clone(),
                    group.model.clone(),
                    total,
                    error_count,
                    actual_ratio,
                ));
            }
        }
        findings
    }

    #[test]
    fn error_ratio_detects_high_error_rate() {
        let mut exchanges = Vec::new();
        for i in 0..10 {
            let is_error = i < 7; // 70% error rate
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: if is_error {
                    None
                } else {
                    Some(format!("res-{i}"))
                },
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: if is_error {
                    None
                } else {
                    Some(i * 1000 + 500)
                },
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: if is_error { None } else { Some(512) },
                response_complete: !is_error,
            });
        }
        let findings = detect_error_ratio(&exchanges, 5, 300);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].3, 7);
        assert!(findings[0].4 >= 300);
    }

    #[test]
    fn error_ratio_no_match_low_error_rate() {
        let mut exchanges = Vec::new();
        for i in 0..10 {
            let is_error = i < 2; // 20% error rate
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: if is_error {
                    None
                } else {
                    Some(format!("res-{i}"))
                },
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: if is_error {
                    None
                } else {
                    Some(i * 1000 + 500)
                },
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: if is_error { None } else { Some(512) },
                response_complete: !is_error,
            });
        }
        // threshold 300 per mille = 30%, actual is 20%
        let findings = detect_error_ratio(&exchanges, 5, 300);
        assert!(findings.is_empty());
    }

    #[test]
    fn error_ratio_respects_minimum_exchanges() {
        let mut exchanges = Vec::new();
        for i in 0..3 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: None,
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: None,
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            });
        }
        // 100% error rate but only 3 exchanges < minimum 5
        let findings = detect_error_ratio(&exchanges, 5, 100);
        assert!(findings.is_empty());
    }

    #[test]
    fn error_ratio_verifies_total_exchanges() {
        let mut exchanges = Vec::new();
        // 20 exchanges, 5 errors = 250‰
        for i in 0..20u64 {
            let is_error = i < 5;
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: if is_error {
                    None
                } else {
                    Some(format!("res-{i}"))
                },
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: if is_error {
                    None
                } else {
                    Some(i * 1000 + 500)
                },
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: if is_error { None } else { Some(512) },
                response_complete: !is_error,
            });
        }
        let findings = detect_error_ratio(&exchanges, 5, 250);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].2, 20); // total_exchanges
        assert_eq!(findings[0].3, 5); // error_count
        assert_eq!(findings[0].4, 250); // actual_ratio_per_mille

        // one per mille above the actual ratio → no finding
        let findings = detect_error_ratio(&exchanges, 5, 251);
        assert!(findings.is_empty());
    }

    // ── Rule 5: Context/token growth detection ───────────────────────

    fn detect_context_growth(
        exchanges: &[MockExchange],
        growth_ratio_per_mille: u64,
        minimum_baseline_bytes: u64,
        minimum_growth_bytes: u64,
        window_size: usize,
        minimum_samples: usize,
    ) -> Vec<(String, u64, u64, u64, u64)> {
        let groups = group_exchanges(exchanges);
        let mut findings = Vec::new();

        for (_group, group_exchanges) in &groups {
            let mut sorted = group_exchanges.clone();
            sorted.sort_by_key(|e| e.started_at);
            let mut history: Vec<u64> = Vec::new();

            for exchange in &sorted {
                let bytes = exchange.request_body_bytes;
                let baseline = if history.len() >= minimum_samples {
                    Some(median(&history))
                } else {
                    None
                };
                if bytes >= minimum_growth_bytes
                    && baseline.is_some_and(|b| {
                        b >= minimum_baseline_bytes
                            && bytes.saturating_sub(b) >= minimum_growth_bytes
                            && u128::from(bytes) * 1000
                                >= u128::from(b) * u128::from(growth_ratio_per_mille)
                    })
                {
                    let b = baseline.unwrap_or(0);
                    let ratio = if b > 0 {
                        let r = u128::from(bytes) * 1000 / u128::from(b);
                        r.min(u128::from(u64::MAX)) as u64
                    } else {
                        0
                    };
                    findings.push((
                        exchange.request_action_id.clone(),
                        bytes,
                        b,
                        ratio,
                        exchange.started_at,
                    ));
                }
                history.push(bytes);
                if history.len() > window_size {
                    history.remove(0);
                }
            }
        }
        findings
    }

    #[test]
    fn context_growth_detects_sudden_jump() {
        let mut exchanges = Vec::new();
        // Stable baseline of 10000, then sudden jump to 50000
        let sizes: Vec<u64> = vec![10000, 10000, 10000, 10000, 10000, 50000];
        for (i, &size) in sizes.iter().enumerate() {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i as u64 * 1000,
                completed_at: Some(i as u64 * 1000 + 500),
                request_body_bytes: size,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        // 50000 / 10000 = 5x = 5000 per mille, threshold 2000
        let findings =
            detect_context_growth(&exchanges, 2000, 8192, 32768, 5, 3);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].0, "req-5");
        assert_eq!(findings[0].1, 50000);
        assert_eq!(findings[0].2, 10000);
        assert_eq!(findings[0].3, 5000);
    }

    #[test]
    fn context_growth_no_match_gradual_increase() {
        let mut exchanges = Vec::new();
        // Gradual 10% increase each time
        let sizes: Vec<u64> = vec![10000, 11000, 12100, 13310, 14641, 16105];
        for (i, &size) in sizes.iter().enumerate() {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i as u64 * 1000,
                completed_at: Some(i as u64 * 1000 + 500),
                request_body_bytes: size,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        // 10% growth = 1100 per mille, threshold 2000
        let findings =
            detect_context_growth(&exchanges, 2000, 8192, 32768, 5, 3);
        assert!(findings.is_empty());
    }

    #[test]
    fn context_growth_respects_minimum_baseline() {
        let mut exchanges = Vec::new();
        // Small baseline, then jump
        let sizes: Vec<u64> = vec![100, 100, 100, 100, 100, 10000];
        for (i, &size) in sizes.iter().enumerate() {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i as u64 * 1000,
                completed_at: Some(i as u64 * 1000 + 500),
                request_body_bytes: size,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        // baseline 100 < minimum_baseline_bytes 8192
        let findings =
            detect_context_growth(&exchanges, 2000, 8192, 32768, 5, 3);
        assert!(findings.is_empty());
    }

    // ── Utility function tests ───────────────────────────────────────

    #[test]
    fn median_odd_count() {
        assert_eq!(median(&[3, 1, 2]), 2);
        assert_eq!(median(&[10, 20, 30]), 20);
        assert_eq!(median(&[1]), 1);
    }

    #[test]
    fn median_even_count() {
        assert_eq!(median(&[1, 2, 3, 4]), 2); // (2+3)/2 = 2
        assert_eq!(median(&[10, 20]), 15);
        assert_eq!(median(&[100, 200, 300, 400]), 250);
    }

    #[test]
    fn median_single_element() {
        assert_eq!(median(&[42]), 42);
    }

    #[test]
    fn similar_requests_exact_match() {
        let a = MockExchange {
            call_action_id: "c1".into(),
            request_action_id: "r1".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 0,
            completed_at: None,
            request_body_bytes: 1024,
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        let b = MockExchange {
            call_action_id: "c2".into(),
            request_action_id: "r2".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 1000,
            completed_at: None,
            request_body_bytes: 1024,
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        assert!(similar_requests(&a, &b, 50));
    }

    #[test]
    fn similar_requests_within_tolerance() {
        let a = MockExchange {
            call_action_id: "c1".into(),
            request_action_id: "r1".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 0,
            completed_at: None,
            request_body_bytes: 1000,
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        let b = MockExchange {
            call_action_id: "c2".into(),
            request_action_id: "r2".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 1000,
            completed_at: None,
            request_body_bytes: 1040, // 4% diff, tolerance 5%
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        assert!(similar_requests(&a, &b, 50)); // 50 per mille = 5%
    }

    #[test]
    fn similar_requests_exceeds_tolerance() {
        let a = MockExchange {
            call_action_id: "c1".into(),
            request_action_id: "r1".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 0,
            completed_at: None,
            request_body_bytes: 1000,
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        let b = MockExchange {
            call_action_id: "c2".into(),
            request_action_id: "r2".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 1000,
            completed_at: None,
            request_body_bytes: 1100, // 10% diff, tolerance 5%
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        assert!(!similar_requests(&a, &b, 50));
    }

    #[test]
    fn similar_requests_different_process() {
        let a = MockExchange {
            call_action_id: "c1".into(),
            request_action_id: "r1".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 0,
            completed_at: None,
            request_body_bytes: 1024,
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        let b = MockExchange {
            call_action_id: "c2".into(),
            request_action_id: "r2".into(),
            response_action_id: None,
            process_id: "p2".into(),
            model: Some("gpt-4".into()),
            started_at: 1000,
            completed_at: None,
            request_body_bytes: 1024,
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        assert!(!similar_requests(&a, &b, 50));
    }

    #[test]
    fn similar_requests_different_model() {
        let a = MockExchange {
            call_action_id: "c1".into(),
            request_action_id: "r1".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 0,
            completed_at: None,
            request_body_bytes: 1024,
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        let b = MockExchange {
            call_action_id: "c2".into(),
            request_action_id: "r2".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("claude-3".into()),
            started_at: 1000,
            completed_at: None,
            request_body_bytes: 1024,
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        assert!(!similar_requests(&a, &b, 50));
    }

    #[test]
    fn similar_requests_zero_tolerance() {
        let a = MockExchange {
            call_action_id: "c1".into(),
            request_action_id: "r1".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 0,
            completed_at: None,
            request_body_bytes: 1024,
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        let b = MockExchange {
            call_action_id: "c2".into(),
            request_action_id: "r2".into(),
            response_action_id: None,
            process_id: "p1".into(),
            model: Some("gpt-4".into()),
            started_at: 1000,
            completed_at: None,
            request_body_bytes: 1025, // 1 byte off
            request_complete: true,
            response_body_bytes: None,
            response_complete: false,
        };
        assert!(!similar_requests(&a, &b, 0));
    }

    // ── Config validation tests ──────────────────────────────────────

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TestConfig {
        high_frequency: TestHighFrequencyRule,
        consecutive_retry: TestConsecutiveRetryRule,
        repeated_similar: TestRepeatedSimilarRule,
        error_ratio: TestErrorRatioRule,
        context_growth: TestContextGrowthRule,
        page_size: u32,
        trace_state_max_count: usize,
        finding_max_count: usize,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct TestHighFrequencyRule {
        enabled: bool,
        window_size_ms: u64,
        threshold: usize,
        min_exchanges: usize,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct TestConsecutiveRetryRule {
        enabled: bool,
        consecutive_count: usize,
        min_request_bytes: usize,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct TestRepeatedSimilarRule {
        enabled: bool,
        similarity_window: usize,
        min_repeat_count: usize,
        similarity_tolerance_ratio_per_mille: u64,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct TestErrorRatioRule {
        enabled: bool,
        minimum_exchanges: usize,
        error_ratio_per_mille: u64,
    }

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct TestContextGrowthRule {
        enabled: bool,
        growth_ratio_per_mille: u64,
        minimum_baseline_bytes: u64,
        minimum_growth_bytes: u64,
        window_size: usize,
        minimum_samples: usize,
    }

    fn validate_config(config: &TestConfig) -> Result<(), String> {
        if config.high_frequency.window_size_ms == 0 {
            return Err("high_frequency.window_size_ms must be greater than zero".into());
        }
        if config.high_frequency.threshold < 2 {
            return Err("high_frequency.threshold must be at least 2".into());
        }
        if config.high_frequency.min_exchanges < 1 {
            return Err("high_frequency.min_exchanges must be at least 1".into());
        }
        if config.consecutive_retry.consecutive_count < 2 {
            return Err("consecutive_retry.consecutive_count must be at least 2".into());
        }
        if config.repeated_similar.similarity_window < 2 {
            return Err("repeated_similar.similarity_window must be at least 2".into());
        }
        if config.repeated_similar.min_repeat_count < 2 {
            return Err("repeated_similar.min_repeat_count must be at least 2".into());
        }
        if config.error_ratio.minimum_exchanges < 1 {
            return Err("error_ratio.minimum_exchanges must be at least 1".into());
        }
        if config.error_ratio.error_ratio_per_mille < 1
            || config.error_ratio.error_ratio_per_mille > 1000
        {
            return Err("error_ratio.error_ratio_per_mille must be between 1 and 1000".into());
        }
        if config.context_growth.growth_ratio_per_mille <= 1000 {
            return Err("context_growth.growth_ratio_per_mille must be greater than 1000".into());
        }
        if config.context_growth.window_size == 0 || config.context_growth.window_size > 64 {
            return Err("context_growth.window_size must be between 1 and 64".into());
        }
        if config.context_growth.minimum_samples == 0
            || config.context_growth.minimum_samples > config.context_growth.window_size
        {
            return Err("context_growth.minimum_samples must be between 1 and window_size".into());
        }
        if config.page_size == 0 || config.page_size > 256 {
            return Err("page_size must be between 1 and 256".into());
        }
        if config.trace_state_max_count == 0 || config.trace_state_max_count > 4096 {
            return Err("trace_state_max_count must be between 1 and 4096".into());
        }
        if config.finding_max_count == 0 || config.finding_max_count > 4096 {
            return Err("finding_max_count must be between 1 and 4096".into());
        }
        Ok(())
    }

    #[test]
    fn config_valid_default() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn config_invalid_threshold_zero() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 1, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_invalid_consecutive_count_one() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 1, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_invalid_growth_ratio() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 1000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_invalid_page_size_zero() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 0,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_invalid_minimum_samples_exceeds_window() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 3, "minimum_samples": 5 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_rejects_unknown_fields() {
        let result = serde_json::from_str::<TestConfig>(
            r#"{
                "unknown_field": true,
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn config_invalid_window_size_zero() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 0, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_invalid_error_ratio_exceeds_max() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 1001 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_invalid_context_growth_window_size_exceeds_max() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 65, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_invalid_minimum_samples_zero() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 0 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_invalid_finding_max_count_zero() {
        let config: TestConfig = serde_json::from_str(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10 },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 0
            }"#,
        )
        .unwrap();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn config_rejects_unknown_rule_fields() {
        // Unknown field nested inside a rule must be rejected, not ignored
        let result = serde_json::from_str::<TestConfig>(
            r#"{
                "high_frequency": { "enabled": true, "window_size_ms": 60000, "threshold": 30, "min_exchanges": 10, "extra_field": true },
                "consecutive_retry": { "enabled": true, "consecutive_count": 3, "min_request_bytes": 0 },
                "repeated_similar": { "enabled": true, "similarity_window": 10, "min_repeat_count": 3, "similarity_tolerance_ratio_per_mille": 50 },
                "error_ratio": { "enabled": true, "minimum_exchanges": 5, "error_ratio_per_mille": 300 },
                "context_growth": { "enabled": true, "growth_ratio_per_mille": 2000, "minimum_baseline_bytes": 8192, "minimum_growth_bytes": 32768, "window_size": 5, "minimum_samples": 3 },
                "page_size": 256,
                "trace_state_max_count": 256,
                "finding_max_count": 100
            }"#,
        );
        assert!(result.is_err());
    }

    // ── Grouping tests ───────────────────────────────────────────────

    #[test]
    fn group_exchanges_separates_by_model() {
        let exchanges = vec![
            MockExchange {
                call_action_id: "c1".into(),
                request_action_id: "r1".into(),
                response_action_id: None,
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: 0,
                completed_at: None,
                request_body_bytes: 100,
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            },
            MockExchange {
                call_action_id: "c2".into(),
                request_action_id: "r2".into(),
                response_action_id: None,
                process_id: "p1".into(),
                model: Some("claude-3".into()),
                started_at: 1000,
                completed_at: None,
                request_body_bytes: 200,
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            },
            MockExchange {
                call_action_id: "c3".into(),
                request_action_id: "r3".into(),
                response_action_id: None,
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: 2000,
                completed_at: None,
                request_body_bytes: 300,
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            },
        ];
        let groups = group_exchanges(&exchanges);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn group_exchanges_separates_by_process() {
        let exchanges = vec![
            MockExchange {
                call_action_id: "c1".into(),
                request_action_id: "r1".into(),
                response_action_id: None,
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: 0,
                completed_at: None,
                request_body_bytes: 100,
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            },
            MockExchange {
                call_action_id: "c2".into(),
                request_action_id: "r2".into(),
                response_action_id: None,
                process_id: "p2".into(),
                model: Some("gpt-4".into()),
                started_at: 1000,
                completed_at: None,
                request_body_bytes: 200,
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            },
        ];
        let groups = group_exchanges(&exchanges);
        assert_eq!(groups.len(), 2);
    }

    // ══════════════════════════════════════════════════════════════════
    //  Scenario: Malicious / Broken Agent — full pipeline test
    //
    //  Simulates a single trace with two concurrent processes (main
    //  agent + spawned sub-agent) that exhibits ALL FIVE anomaly types
    //  simultaneously, then verifies detection results and payload
    //  serialization for each rule.
    //
    //  Timeline (ms):
    //  ─────────────────────────────────────────────────────────────
    //  Process "main-agent" (model: gpt-4)
    //    t=0     req-0   10KB   OK      ← baseline
    //    t=1000  req-1   10KB   OK      ← baseline
    //    t=2000  req-2   10KB   OK      ← baseline
    //    t=3000  req-3   10KB   OK      ← baseline
    //    t=4000  req-4   10KB   OK      ← baseline (min_samples=3 met)
    //    t=5000  req-5   10KB   OK      ← baseline
    //    t=6000  req-6   60KB   OK      ← RULE 5: context growth (6x)
    //    t=7000  req-7   10KB   OK
    //    t=8000  req-8   10KB   OK
    //    t=9000  req-9   10KB   OK
    //    t=10000 req-10  10KB   OK
    //    t=11000 req-11  10KB   OK
    //    t=12000 req-12  10KB   OK
    //    t=13000 req-13  10KB   OK
    //    t=14000 req-14  10KB   OK
    //    t=15000 req-15  10KB   OK
    //    t=16000 req-16  10KB   OK
    //    t=17000 req-17  10KB   OK
    //    t=18000 req-18  10KB   OK
    //    t=19000 req-19  10KB   OK
    //    t=20000 req-20  10KB   OK
    //    t=21000 req-21  10KB   OK
    //    t=22000 req-22  10KB   OK
    //    t=23000 req-23  10KB   OK
    //    t=24000 req-24  10KB   OK
    //    t=25000 req-25  10KB   OK
    //    t=26000 req-26  10KB   OK
    //    t=27000 req-27  10KB   OK
    //    t=28000 req-28  10KB   OK
    //    t=29000 req-29  10KB   OK
    //    t=30000 req-30  10KB   OK     ← 31 exchanges in 30s window
    //                                     RULE 1: high frequency (31 ≥ 30)
    //
    //  Process "sub-agent" (model: gpt-4)
    //    t=5000  req-s0  5KB    FAIL
    //    t=5500  req-s1  5KB    FAIL
    //    t=6000  req-s2  5KB    FAIL
    //    t=6500  req-s3  5KB    FAIL   ← RULE 2: 4 consecutive retries
    //    t=7000  req-s4  5KB    OK
    //    t=7500  req-s5  5KB    FAIL
    //    t=8000  req-s6  5KB    FAIL
    //    t=8500  req-s7  5KB    FAIL
    //    t=9000  req-s8  5KB    FAIL   ← RULE 2: another run of 4
    //    t=9500  req-s9  5KB    OK
    //    t=10000 req-s10 5KB    FAIL
    //    t=10500 req-s11 5KB    FAIL
    //    t=11000 req-s12 5KB    FAIL
    //    t=11500 req-s13 5KB    FAIL
    //    t=12000 req-s14 5KB    FAIL   ← RULE 2: run of 5
    //    t=12500 req-s15 5KB    OK
    //    t=13000 req-s16 5KB    FAIL
    //    t=13500 req-s17 5KB    FAIL
    //    t=14000 req-s18 5KB    FAIL
    //    t=14500 req-s19 5KB    FAIL   ← RULE 2: run of 4
    //    t=15000 req-s20 5KB    OK
    //    → 21 total, 18 errors → RULE 4: error ratio 18/21 = 857‰
    //
    //  Process "sub-agent" (model: gpt-4) — repeated similar block
    //    t=30000 req-s21 8KB    OK
    //    t=30500 req-s22 8KB    OK
    //    t=31000 req-s23 8KB    OK
    //    t=31500 req-s24 8KB    OK
    //    t=32000 req-s25 8KB    OK
    //    t=32500 req-s26 8KB    OK
    //    t=33000 req-s27 8KB    OK
    //    t=33500 req-s28 8KB    OK
    //    t=34000 req-s29 8KB    OK
    //    t=34500 req-s30 8KB    OK
    //    → RULE 3: 10 identical 8KB requests (repeat=10 ≥ min=3)
    // ══════════════════════════════════════════════════════════════════

    fn build_scenario_exchanges() -> Vec<MockExchange> {
        let mut e = Vec::new();

        // ── main-agent: 31 normal requests + 1 growth spike ──
        for i in 0u64..31 {
            let bytes = if i == 6 { 60_000 } else { 10_000 };
            e.push(MockExchange {
                call_action_id: format!("main-call-{i}"),
                request_action_id: format!("main-req-{i}"),
                response_action_id: Some(format!("main-res-{i}")),
                process_id: "main-agent".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: Some(i * 1000 + 300),
                request_body_bytes: bytes,
                request_complete: true,
                response_body_bytes: Some(2048),
                response_complete: true,
            });
        }

        // ── sub-agent: error-heavy with consecutive retries ──
        let sub_errors: &[(u64, bool)] = &[
            // first burst: 4 errors
            (5000, false), (5500, false), (6000, false), (6500, false),
            // recovery
            (7000, true),
            // second burst: 4 errors
            (7500, false), (8000, false), (8500, false), (9000, false),
            // recovery
            (9500, true),
            // third burst: 5 errors
            (10000, false), (10500, false), (11000, false), (11500, false),
            (12000, false),
            // recovery
            (12500, true),
            // fourth burst: 4 errors
            (13000, false), (13500, false), (14000, false), (14500, false),
            // recovery
            (15000, true),
        ];
        for (i, &(ts, ok)) in sub_errors.iter().enumerate() {
            e.push(MockExchange {
                call_action_id: format!("sub-call-{i}"),
                request_action_id: format!("sub-req-{i}"),
                response_action_id: if ok {
                    Some(format!("sub-res-{i}"))
                } else {
                    None
                },
                process_id: "sub-agent".into(),
                model: Some("gpt-4".into()),
                started_at: ts,
                completed_at: if ok { Some(ts + 200) } else { None },
                request_body_bytes: 5000,
                request_complete: true,
                response_body_bytes: if ok { Some(1024) } else { None },
                response_complete: ok,
            });
        }

        // ── sub-agent: repeated similar block (10 × 8KB) ──
        for i in 0u64..10 {
            e.push(MockExchange {
                call_action_id: format!("sub-sim-call-{i}"),
                request_action_id: format!("sub-sim-req-{i}"),
                response_action_id: Some(format!("sub-sim-res-{i}")),
                process_id: "sub-agent".into(),
                model: Some("gpt-4".into()),
                started_at: 30_000 + i * 500,
                completed_at: Some(30_000 + i * 500 + 200),
                request_body_bytes: 8000,
                request_complete: true,
                response_body_bytes: Some(1024),
                response_complete: true,
            });
        }

        e
    }

    #[test]
    fn scenario_all_rules_fire() {
        let exchanges = build_scenario_exchanges();
        let _groups = group_exchanges(&exchanges);

        // ── Rule 1: high frequency ──
        // main-agent: 31 exchanges in 30s, window=30s, threshold=30
        let hf = detect_high_frequency(&exchanges, 30_000, 30, 10);
        assert!(
            !hf.is_empty(),
            "Rule 1 (high frequency) should fire for main-agent"
        );
        let main_hf = hf.iter().find(|f| f.0 == "main-agent");
        assert!(main_hf.is_some(), "main-agent should have high-frequency finding");
        assert!(main_hf.unwrap().2 >= 30);

        // sub-agent has 21+10=31 exchanges, also qualifies
        let sub_hf = hf.iter().find(|f| f.0 == "sub-agent");
        assert!(sub_hf.is_some(), "sub-agent should also have high-frequency finding");

        // ── Rule 2: consecutive retry ──
        // sub-agent has 4 bursts of consecutive errors (4,4,5,4)
        let cr = detect_consecutive_retry(&exchanges, 3, 0);
        assert!(
            cr.len() >= 4,
            "Rule 2 (consecutive retry) should find ≥4 bursts, got {}",
            cr.len()
        );
        // all findings should be for sub-agent
        for f in &cr {
            assert_eq!(f.0, "sub-agent");
            assert!(f.2 >= 3, "retry_length should be ≥3, got {}", f.2);
        }

        // ── Rule 3: repeated similar ──
        // sub-agent has 10 identical 8KB requests at t=30000..34500
        let rs = detect_repeated_similar(&exchanges, 10, 3, 50);
        assert!(
            !rs.is_empty(),
            "Rule 3 (repeated similar) should fire for sub-agent"
        );
        let sub_rs = rs.iter().find(|f| f.0 == "sub-agent");
        assert!(sub_rs.is_some());
        assert!(sub_rs.unwrap().2 >= 3, "repeat_count should be ≥3");

        // ── Rule 4: error ratio ──
        // sub-agent: 21 error-phase exchanges, 18 errors → 857‰
        let er = detect_error_ratio(&exchanges, 5, 300);
        assert!(
            !er.is_empty(),
            "Rule 4 (error ratio) should fire for sub-agent"
        );
        let sub_er = er.iter().find(|f| f.0 == "sub-agent");
        assert!(sub_er.is_some());
        assert!(sub_er.unwrap().4 >= 300, "actual_ratio should be ≥300");

        // main-agent has 0 errors → should NOT appear
        let main_er = er.iter().find(|f| f.0 == "main-agent");
        assert!(main_er.is_none(), "main-agent should not have error ratio finding");

        // ── Rule 5: context growth ──
        // main-agent: baseline ~10KB, spike to 60KB at req-6
        let cg = detect_context_growth(&exchanges, 2000, 8192, 32768, 5, 3);
        assert!(
            !cg.is_empty(),
            "Rule 5 (context growth) should fire for main-agent"
        );
        let main_cg = cg.iter().find(|f| f.0 == "main-req-6");
        assert!(main_cg.is_some(), "main-req-6 should be the growth finding");
        assert_eq!(main_cg.unwrap().1, 60_000); // observed_bytes
        assert_eq!(main_cg.unwrap().2, 10_000); // baseline
        assert_eq!(main_cg.unwrap().3, 6000); // ratio 60000*1000/10000 = 6000
    }

    #[test]
    fn scenario_payload_serialization() {
        use serde_json;

        // ── High frequency payload ──
        #[derive(serde::Serialize)]
        struct HfPayload {
            root_container_id: Option<String>,
            root_process_id: String,
            display_name: String,
            profile_name: String,
            window_size_ms: u64,
            threshold: usize,
            findings: Vec<serde_json::Value>,
            truncated_count: usize,
        }
        let hf_payload = HfPayload {
            root_container_id: None,
            root_process_id: "agent-root-1234".into(),
            display_name: "malicious-agent-test".into(),
            profile_name: "default".into(),
            window_size_ms: 60_000,
            threshold: 30,
            findings: vec![serde_json::json!({
                "process_id": "main-agent",
                "model": "gpt-4",
                "exchange_count": 31,
                "window_start_ms": 0,
                "window_end_ms": 30_000
            })],
            truncated_count: 0,
        };
        let json = serde_json::to_string(&hf_payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["root_process_id"], "agent-root-1234");
        assert_eq!(parsed["display_name"], "malicious-agent-test");
        assert_eq!(parsed["findings"][0]["exchange_count"], 31);
        assert_eq!(parsed["truncated_count"], 0);

        // ── Consecutive retry payload ──
        #[derive(serde::Serialize)]
        struct CrPayload {
            root_container_id: Option<String>,
            root_process_id: String,
            display_name: String,
            profile_name: String,
            consecutive_count: usize,
            findings: Vec<serde_json::Value>,
            truncated_count: usize,
        }
        let cr_payload = CrPayload {
            root_container_id: Some("container-abc".into()),
            root_process_id: "agent-root-1234".into(),
            display_name: "malicious-agent-test".into(),
            profile_name: "default".into(),
            consecutive_count: 3,
            findings: vec![serde_json::json!({
                "process_id": "sub-agent",
                "model": "gpt-4",
                "retry_length": 5,
                "first_action_id": "sub-req-10",
                "last_action_id": "sub-req-14",
                "first_started_at_ms": 10000,
                "last_started_at_ms": 12000
            })],
            truncated_count: 0,
        };
        let json = serde_json::to_string(&cr_payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["root_container_id"], "container-abc");
        assert_eq!(parsed["findings"][0]["retry_length"], 5);
        assert_eq!(parsed["findings"][0]["first_action_id"], "sub-req-10");

        // ── Repeated similar payload ──
        #[derive(serde::Serialize)]
        struct RsPayload {
            root_container_id: Option<String>,
            root_process_id: String,
            display_name: String,
            profile_name: String,
            similarity_window: usize,
            similarity_tolerance_ratio_per_mille: u64,
            min_repeat_count: usize,
            findings: Vec<serde_json::Value>,
            truncated_count: usize,
        }
        let rs_payload = RsPayload {
            root_container_id: None,
            root_process_id: "agent-root-1234".into(),
            display_name: "malicious-agent-test".into(),
            profile_name: "default".into(),
            similarity_window: 10,
            similarity_tolerance_ratio_per_mille: 50,
            min_repeat_count: 3,
            findings: vec![serde_json::json!({
                "process_id": "sub-agent",
                "model": "gpt-4",
                "repeat_count": 10,
                "representative_action_id": "sub-sim-req-0",
                "representative_request_bytes": 8000,
                "first_started_at_ms": 30000,
                "last_started_at_ms": 34500
            })],
            truncated_count: 0,
        };
        let json = serde_json::to_string(&rs_payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["min_repeat_count"], 3);
        assert_eq!(parsed["findings"][0]["repeat_count"], 10);
        assert_eq!(parsed["findings"][0]["representative_request_bytes"], 8000);

        // ── Error ratio payload ──
        #[derive(serde::Serialize)]
        struct ErPayload {
            root_container_id: Option<String>,
            root_process_id: String,
            display_name: String,
            profile_name: String,
            minimum_exchanges: usize,
            error_ratio_per_mille: u64,
            findings: Vec<serde_json::Value>,
            truncated_count: usize,
        }
        let er_payload = ErPayload {
            root_container_id: None,
            root_process_id: "agent-root-1234".into(),
            display_name: "malicious-agent-test".into(),
            profile_name: "default".into(),
            minimum_exchanges: 5,
            error_ratio_per_mille: 300,
            findings: vec![serde_json::json!({
                "process_id": "sub-agent",
                "model": "gpt-4",
                "total_exchanges": 21,
                "error_count": 18,
                "actual_ratio_per_mille": 857
            })],
            truncated_count: 0,
        };
        let json = serde_json::to_string(&er_payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["findings"][0]["error_count"], 18);
        assert_eq!(parsed["findings"][0]["actual_ratio_per_mille"], 857);

        // ── Context growth payload ──
        #[derive(serde::Serialize)]
        struct CgPayload {
            root_container_id: Option<String>,
            root_process_id: String,
            display_name: String,
            profile_name: String,
            growth_ratio_per_mille: u64,
            minimum_baseline_bytes: u64,
            minimum_growth_bytes: u64,
            window_size: usize,
            minimum_samples: usize,
            findings: Vec<serde_json::Value>,
            truncated_count: usize,
        }
        let cg_payload = CgPayload {
            root_container_id: None,
            root_process_id: "agent-root-1234".into(),
            display_name: "malicious-agent-test".into(),
            profile_name: "default".into(),
            growth_ratio_per_mille: 2000,
            minimum_baseline_bytes: 8192,
            minimum_growth_bytes: 32768,
            window_size: 5,
            minimum_samples: 3,
            findings: vec![serde_json::json!({
                "action_id": "main-req-6",
                "call_action_id": "main-call-6",
                "process_id": "main-agent",
                "model": "gpt-4",
                "observed_bytes": 60000,
                "baseline_median_bytes": 10000,
                "observed_ratio_per_mille": 6000,
                "started_at_ms": 6000
            })],
            truncated_count: 0,
        };
        let json = serde_json::to_string(&cg_payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["findings"][0]["observed_bytes"], 60000);
        assert_eq!(parsed["findings"][0]["baseline_median_bytes"], 10000);
        assert_eq!(parsed["findings"][0]["observed_ratio_per_mille"], 6000);
    }

    #[test]
    fn payload_field_completeness() {
        // Each payload mirrors the exact field set of the plugin's Serialize
        // structs, then round-trips through serde_json to verify the contract.

        // ── High frequency payload ──
        let hf = serde_json::json!({
            "root_container_id": null,
            "root_process_id": "agent-root-1234",
            "display_name": "agent-a",
            "profile_name": "default",
            "window_size_ms": 60_000,
            "threshold": 30,
            "findings": [{
                "process_id": "p1",
                "model": "gpt-4",
                "exchange_count": 31,
                "window_start_ms": 0,
                "window_end_ms": 30_000
            }],
            "truncated_count": 0
        });
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&hf).unwrap()).unwrap();
        assert!(parsed["root_container_id"].is_null());
        assert_eq!(parsed["root_process_id"], "agent-root-1234");
        assert_eq!(parsed["display_name"], "agent-a");
        assert_eq!(parsed["profile_name"], "default");
        assert_eq!(parsed["window_size_ms"], 60_000);
        assert_eq!(parsed["threshold"], 30);
        assert_eq!(parsed["findings"][0]["process_id"], "p1");
        assert_eq!(parsed["findings"][0]["model"], "gpt-4");
        assert_eq!(parsed["findings"][0]["exchange_count"], 31);
        assert_eq!(parsed["findings"][0]["window_start_ms"], 0);
        assert_eq!(parsed["findings"][0]["window_end_ms"], 30_000);
        assert_eq!(parsed["truncated_count"], 0);

        // ── Consecutive retry payload ──
        let cr = serde_json::json!({
            "root_container_id": "container-abc",
            "root_process_id": "agent-root-1234",
            "display_name": "agent-a",
            "profile_name": "default",
            "consecutive_count": 3,
            "findings": [{
                "process_id": "sub-agent",
                "model": "gpt-4",
                "retry_length": 5,
                "first_action_id": "sub-req-10",
                "last_action_id": "sub-req-14",
                "first_started_at_ms": 10_000,
                "last_started_at_ms": 12_000
            }],
            "truncated_count": 0
        });
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&cr).unwrap()).unwrap();
        assert_eq!(parsed["root_container_id"], "container-abc");
        assert_eq!(parsed["root_process_id"], "agent-root-1234");
        assert_eq!(parsed["display_name"], "agent-a");
        assert_eq!(parsed["profile_name"], "default");
        assert_eq!(parsed["consecutive_count"], 3);
        assert_eq!(parsed["findings"][0]["process_id"], "sub-agent");
        assert_eq!(parsed["findings"][0]["model"], "gpt-4");
        assert_eq!(parsed["findings"][0]["retry_length"], 5);
        assert_eq!(parsed["findings"][0]["first_action_id"], "sub-req-10");
        assert_eq!(parsed["findings"][0]["last_action_id"], "sub-req-14");
        assert_eq!(parsed["findings"][0]["first_started_at_ms"], 10_000);
        assert_eq!(parsed["findings"][0]["last_started_at_ms"], 12_000);
        assert_eq!(parsed["truncated_count"], 0);

        // ── Repeated similar payload ──
        let rs = serde_json::json!({
            "root_container_id": null,
            "root_process_id": "agent-root-1234",
            "display_name": "agent-a",
            "profile_name": "default",
            "similarity_window": 10,
            "similarity_tolerance_ratio_per_mille": 50,
            "min_repeat_count": 3,
            "findings": [{
                "process_id": "sub-agent",
                "model": "gpt-4",
                "repeat_count": 10,
                "representative_action_id": "sub-sim-req-0",
                "representative_request_bytes": 8000,
                "first_started_at_ms": 30_000,
                "last_started_at_ms": 34_500
            }],
            "truncated_count": 0
        });
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&rs).unwrap()).unwrap();
        assert!(parsed["root_container_id"].is_null());
        assert_eq!(parsed["root_process_id"], "agent-root-1234");
        assert_eq!(parsed["display_name"], "agent-a");
        assert_eq!(parsed["profile_name"], "default");
        assert_eq!(parsed["similarity_window"], 10);
        assert_eq!(parsed["similarity_tolerance_ratio_per_mille"], 50);
        assert_eq!(parsed["min_repeat_count"], 3);
        assert_eq!(parsed["findings"][0]["process_id"], "sub-agent");
        assert_eq!(parsed["findings"][0]["model"], "gpt-4");
        assert_eq!(parsed["findings"][0]["repeat_count"], 10);
        assert_eq!(parsed["findings"][0]["representative_action_id"], "sub-sim-req-0");
        assert_eq!(parsed["findings"][0]["representative_request_bytes"], 8000);
        assert_eq!(parsed["findings"][0]["first_started_at_ms"], 30_000);
        assert_eq!(parsed["findings"][0]["last_started_at_ms"], 34_500);
        assert_eq!(parsed["truncated_count"], 0);

        // ── Error ratio payload ──
        let er = serde_json::json!({
            "root_container_id": null,
            "root_process_id": "agent-root-1234",
            "display_name": "agent-a",
            "profile_name": "default",
            "minimum_exchanges": 5,
            "error_ratio_per_mille": 300,
            "findings": [{
                "process_id": "sub-agent",
                "model": "gpt-4",
                "total_exchanges": 21,
                "error_count": 18,
                "actual_ratio_per_mille": 857
            }],
            "truncated_count": 0
        });
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&er).unwrap()).unwrap();
        assert!(parsed["root_container_id"].is_null());
        assert_eq!(parsed["root_process_id"], "agent-root-1234");
        assert_eq!(parsed["display_name"], "agent-a");
        assert_eq!(parsed["profile_name"], "default");
        assert_eq!(parsed["minimum_exchanges"], 5);
        assert_eq!(parsed["error_ratio_per_mille"], 300);
        assert_eq!(parsed["findings"][0]["process_id"], "sub-agent");
        assert_eq!(parsed["findings"][0]["model"], "gpt-4");
        assert_eq!(parsed["findings"][0]["total_exchanges"], 21);
        assert_eq!(parsed["findings"][0]["error_count"], 18);
        assert_eq!(parsed["findings"][0]["actual_ratio_per_mille"], 857);
        assert_eq!(parsed["truncated_count"], 0);

        // ── Context growth payload ──
        let cg = serde_json::json!({
            "root_container_id": "container-abc",
            "root_process_id": "agent-root-1234",
            "display_name": "agent-a",
            "profile_name": "default",
            "growth_ratio_per_mille": 2000,
            "minimum_baseline_bytes": 8192,
            "minimum_growth_bytes": 32_768,
            "window_size": 5,
            "minimum_samples": 3,
            "findings": [{
                "action_id": "main-req-6",
                "call_action_id": "main-call-6",
                "process_id": "main-agent",
                "model": "gpt-4",
                "observed_bytes": 60_000,
                "baseline_median_bytes": 10_000,
                "observed_ratio_per_mille": 6000,
                "started_at_ms": 6000
            }],
            "truncated_count": 0
        });
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&cg).unwrap()).unwrap();
        assert_eq!(parsed["root_container_id"], "container-abc");
        assert_eq!(parsed["root_process_id"], "agent-root-1234");
        assert_eq!(parsed["display_name"], "agent-a");
        assert_eq!(parsed["profile_name"], "default");
        assert_eq!(parsed["growth_ratio_per_mille"], 2000);
        assert_eq!(parsed["minimum_baseline_bytes"], 8192);
        assert_eq!(parsed["minimum_growth_bytes"], 32_768);
        assert_eq!(parsed["window_size"], 5);
        assert_eq!(parsed["minimum_samples"], 3);
        assert_eq!(parsed["findings"][0]["action_id"], "main-req-6");
        assert_eq!(parsed["findings"][0]["call_action_id"], "main-call-6");
        assert_eq!(parsed["findings"][0]["process_id"], "main-agent");
        assert_eq!(parsed["findings"][0]["model"], "gpt-4");
        assert_eq!(parsed["findings"][0]["observed_bytes"], 60_000);
        assert_eq!(parsed["findings"][0]["baseline_median_bytes"], 10_000);
        assert_eq!(parsed["findings"][0]["observed_ratio_per_mille"], 6000);
        assert_eq!(parsed["findings"][0]["started_at_ms"], 6000);
        assert_eq!(parsed["truncated_count"], 0);
    }

    #[test]
    fn scenario_multi_model_independent_detection() {
        // Two models on the same process: only one should trigger
        let mut exchanges = Vec::new();

        // gpt-4: 35 requests in 30s → triggers high frequency
        for i in 0..35u64 {
            exchanges.push(MockExchange {
                call_action_id: format!("g4-call-{i}"),
                request_action_id: format!("g4-req-{i}"),
                response_action_id: Some(format!("g4-res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 800,
                completed_at: Some(i * 800 + 200),
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }

        // claude-3: 5 requests spread over 100s → does NOT trigger
        for i in 0..5u64 {
            exchanges.push(MockExchange {
                call_action_id: format!("c3-call-{i}"),
                request_action_id: format!("c3-req-{i}"),
                response_action_id: Some(format!("c3-res-{i}")),
                process_id: "p1".into(),
                model: Some("claude-3".into()),
                started_at: i * 20_000,
                completed_at: Some(i * 20_000 + 200),
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }

        let hf = detect_high_frequency(&exchanges, 30_000, 30, 10);
        // sliding window finds multiple overlapping positions where count ≥ 30
        assert!(!hf.is_empty());
        assert!(hf.iter().all(|f| f.0 == "p1"));
        assert!(hf.iter().all(|f| f.1 == Some("gpt-4".into())));
        // claude-3 should not appear
        assert!(!hf.iter().any(|f| f.1 == Some("claude-3".into())));
    }

    #[test]
    fn scenario_empty_exchanges_no_findings() {
        let exchanges: Vec<MockExchange> = vec![];
        let hf = detect_high_frequency(&exchanges, 30_000, 30, 10);
        let cr = detect_consecutive_retry(&exchanges, 3, 0);
        let rs = detect_repeated_similar(&exchanges, 10, 3, 50);
        let er = detect_error_ratio(&exchanges, 5, 300);
        let cg = detect_context_growth(&exchanges, 2000, 8192, 32768, 5, 3);
        assert!(hf.is_empty());
        assert!(cr.is_empty());
        assert!(rs.is_empty());
        assert!(er.is_empty());
        assert!(cg.is_empty());
    }

    #[test]
    fn scenario_rules_disabled_via_threshold() {
        // Set threshold impossibly high to simulate disabled rules
        let exchanges = build_scenario_exchanges();

        let hf = detect_high_frequency(&exchanges, 30_000, 9999, 10);
        assert!(hf.is_empty());

        let cr = detect_consecutive_retry(&exchanges, 9999, 0);
        assert!(cr.is_empty());

        let rs = detect_repeated_similar(&exchanges, 9999, 9999, 50);
        assert!(rs.is_empty());

        let er = detect_error_ratio(&exchanges, 9999, 300);
        assert!(er.is_empty());

        let cg = detect_context_growth(&exchanges, 2000, u64::MAX, u64::MAX, 5, 3);
        assert!(cg.is_empty());
    }

    #[test]
    fn scenario_boundary_consecutive_retry_exactly_at_threshold() {
        let mut exchanges = Vec::new();
        // exactly 3 failures
        for i in 0..3u64 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: None,
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: None,
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: None,
                response_complete: false,
            });
        }
        // threshold = 3 → should fire
        let findings = detect_consecutive_retry(&exchanges, 3, 0);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].2, 3);

        // threshold = 4 → should NOT fire
        let findings = detect_consecutive_retry(&exchanges, 4, 0);
        assert!(findings.is_empty());
    }

    #[test]
    fn scenario_boundary_high_frequency_sliding_window() {
        let mut exchanges = Vec::new();
        // 25 requests in first 25s, then gap, then 10 more → only 25 in any 30s window
        for i in 0..25u64 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: Some(i * 1000 + 200),
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        // big gap
        for i in 25..35u64 {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000 + 60_000, // +60s gap
                completed_at: Some(i * 1000 + 60_000 + 200),
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }

        // threshold=26, window=30s → only 25 in first window → no finding
        let findings = detect_high_frequency(&exchanges, 30_000, 26, 10);
        assert!(findings.is_empty());

        // threshold=25 → fires
        let findings = detect_high_frequency(&exchanges, 30_000, 25, 10);
        assert!(!findings.is_empty());
    }

    #[test]
    fn scenario_boundary_context_growth_insufficient_samples() {
        let mut exchanges = Vec::new();
        // only 2 samples before spike → minimum_samples=3 not met
        let sizes: Vec<u64> = vec![10_000, 10_000, 60_000];
        for (i, &size) in sizes.iter().enumerate() {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i as u64 * 1000,
                completed_at: Some(i as u64 * 1000 + 200),
                request_body_bytes: size,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        // minimum_samples=3 but only 2 history samples at spike point
        let findings = detect_context_growth(&exchanges, 2000, 8192, 32768, 5, 3);
        assert!(findings.is_empty());
    }

    #[test]
    fn scenario_boundary_error_ratio_exact_threshold() {
        let mut exchanges = Vec::new();
        // 10 exchanges: 3 errors = 300‰ exactly
        for i in 0..10u64 {
            let is_error = i < 3;
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: if is_error {
                    None
                } else {
                    Some(format!("res-{i}"))
                },
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i * 1000,
                completed_at: if is_error {
                    None
                } else {
                    Some(i * 1000 + 200)
                },
                request_body_bytes: 1024,
                request_complete: true,
                response_body_bytes: if is_error { None } else { Some(512) },
                response_complete: !is_error,
            });
        }
        // threshold=300 → exactly matches
        let findings = detect_error_ratio(&exchanges, 5, 300);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].4, 300);

        // threshold=301 → just above → no finding
        let findings = detect_error_ratio(&exchanges, 5, 301);
        assert!(findings.is_empty());
    }

    #[test]
    fn scenario_boundary_repeated_similar_tolerance_edge() {
        let mut exchanges = Vec::new();
        // 5 requests: 4 at 1000 bytes, 1 at 1045 bytes (4.5% diff)
        let sizes: Vec<u64> = vec![1000, 1000, 1000, 1000, 1045];
        for (i, &size) in sizes.iter().enumerate() {
            exchanges.push(MockExchange {
                call_action_id: format!("call-{i}"),
                request_action_id: format!("req-{i}"),
                response_action_id: Some(format!("res-{i}")),
                process_id: "p1".into(),
                model: Some("gpt-4".into()),
                started_at: i as u64 * 1000,
                completed_at: Some(i as u64 * 1000 + 200),
                request_body_bytes: size,
                request_complete: true,
                response_body_bytes: Some(512),
                response_complete: true,
            });
        }
        // tolerance=50 (5%) → 1045 is within 5% of 1000 → 5 similar
        let findings = detect_repeated_similar(&exchanges, 5, 3, 50);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].2, 5);

        // tolerance=40 (4%) → 1045 is NOT within 4% → only 4 similar
        let findings = detect_repeated_similar(&exchanges, 5, 3, 40);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].2, 4);
    }
}
