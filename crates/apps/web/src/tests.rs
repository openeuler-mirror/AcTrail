#[cfg(test)]
mod tests {
    use crate::trace_data::*;
    use crate::analysis::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn create_test_trace_document() -> TraceDocument {
        TraceDocument {
            schema_version: "test-v1".to_string(),
            trace_id: "test-trace-1".to_string(),
            completeness: "Complete".to_string(),
            nodes: vec![
                Node {
                    id: "process:1000:1000000".to_string(),
                    kind: "Process".to_string(),
                    title: "pid 1000".to_string(),
                    attributes: BTreeMap::from([
                        ("pid".to_string(), "1000".to_string()),
                        ("generation".to_string(), "1000000".to_string()),
                        ("start_time_ticks".to_string(), "1000000".to_string()),
                        ("state".to_string(), "Active".to_string()),
                        ("capture_enabled".to_string(), "true".to_string()),
                    ]),
                },
                Node {
                    id: "process:1001:2000000".to_string(),
                    kind: "Process".to_string(),
                    title: "pid 1001".to_string(),
                    attributes: BTreeMap::from([
                        ("pid".to_string(), "1001".to_string()),
                        ("generation".to_string(), "2000000".to_string()),
                        ("start_time_ticks".to_string(), "2000000".to_string()),
                        ("state".to_string(), "Exited".to_string()),
                        ("exit_code".to_string(), "0".to_string()),
                        ("exit_observed_at_unix_seconds".to_string(), "1000".to_string()),
                        ("inherited_from_pid".to_string(), "1000".to_string()),
                        ("capture_enabled".to_string(), "true".to_string()),
                    ]),
                },
                Node {
                    id: "process:1002:3000000".to_string(),
                    kind: "Process".to_string(),
                    title: "pid 1002".to_string(),
                    attributes: BTreeMap::from([
                        ("pid".to_string(), "1002".to_string()),
                        ("generation".to_string(), "3000000".to_string()),
                        ("start_time_ticks".to_string(), "3000000".to_string()),
                        ("state".to_string(), "Exited".to_string()),
                        ("exit_code".to_string(), "1".to_string()),
                        ("exit_observed_at_unix_seconds".to_string(), "2000".to_string()),
                        ("inherited_from_pid".to_string(), "1000".to_string()),
                        ("capture_enabled".to_string(), "true".to_string()),
                    ]),
                },
            ],
            edges: vec![
                Edge {
                    source: "process:1000:1000000".to_string(),
                    target: "process:1001:2000000".to_string(),
                    kind: "ProcessSpawned".to_string(),
                    attributes: BTreeMap::new(),
                },
                Edge {
                    source: "process:1000:1000000".to_string(),
                    target: "process:1002:3000000".to_string(),
                    kind: "ProcessSpawned".to_string(),
                    attributes: BTreeMap::new(),
                },
            ],
        }
    }

    #[test]
    fn test_analyze_trace_process_lifetimes() {
        let document = create_test_trace_document();
        let analysis = analyze_trace(&document);

        assert_eq!(analysis.process_lifetimes.len(), 3);
        
        let active_count = analysis.process_lifetimes.iter()
            .filter(|p| p.state == "Active")
            .count();
        assert_eq!(active_count, 1);
        
        let exited_count = analysis.process_lifetimes.iter()
            .filter(|p| p.state == "Exited")
            .count();
        assert_eq!(exited_count, 2);
        
        let with_duration = analysis.process_lifetimes.iter()
            .filter(|p| p.duration_ms.is_some())
            .count();
        assert_eq!(with_duration, 2);
    }

    #[test]
    fn test_analyze_trace_summary() {
        let document = create_test_trace_document();
        let analysis = analyze_trace(&document);

        assert_eq!(analysis.summary.total_processes, 3);
        assert_eq!(analysis.summary.active_processes, 1);
        assert_eq!(analysis.summary.exited_processes, 2);
        assert!(analysis.summary.avg_process_duration_ms.is_some());
        assert!(analysis.summary.max_process_duration_ms.is_some());
        assert!(analysis.summary.min_process_duration_ms.is_some());
    }

    #[test]
    fn test_get_process_hierarchy() {
        let document = create_test_trace_document();
        let analysis = analyze_trace(&document);
        let tree = get_process_hierarchy(&analysis);

        assert_eq!(tree.len(), 1);
        let root = &tree[0];
        assert_eq!(root.pid, 1000);
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.depth, 0);
    }

    #[test]
    fn test_get_slowest_processes() {
        let document = create_test_trace_document();
        let analysis = analyze_trace(&document);
        let slowest = get_slowest_processes(&analysis, 2);

        assert_eq!(slowest.len(), 2);
        assert!(slowest[0].duration_ms.unwrap() >= slowest[1].duration_ms.unwrap());
    }

    #[test]
    fn test_format_duration_ms() {
        assert_eq!(format_duration_ms(Some(500.0)), "500.00ms");
        assert_eq!(format_duration_ms(Some(1500.0)), "1.50s");
        assert_eq!(format_duration_ms(Some(0.5)), "500.00μs");
        assert_eq!(format_duration_ms(None), "N/A");
    }

    #[test]
    fn test_format_ticks() {
        assert_eq!(format_ticks(1_500_000_000), "1.50s");
        assert_eq!(format_ticks(1_500_000), "1.50ms");
        assert_eq!(format_ticks(1000), "1000 ticks");
    }

    #[test]
    fn test_load_real_trace() {
        let trace_path = PathBuf::from("/home/lw/data/trace.json");
        if trace_path.exists() {
            let result = load_trace(&trace_path);
            assert!(result.is_ok());
            
            let document = result.unwrap();
            assert!(!document.trace_id.is_empty());
            assert!(!document.nodes.is_empty());
            
            let analysis = analyze_trace(&document);
            assert!(analysis.summary.total_processes > 0);
        }
    }
}
