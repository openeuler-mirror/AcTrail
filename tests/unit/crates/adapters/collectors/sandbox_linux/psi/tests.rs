// Tests for the corresponding production module.
use super::*;

#[test]
fn parses_kernel_memory_pressure() {
    let raw = "some avg10=2.35 avg60=1.10 avg300=0.50 total=12345\n\
               full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
    let (some, full) = parse_memory_pressure(raw).expect("parse");
    assert_eq!(
        some,
        PsiAverages {
            avg10_millipercent: 2350,
            avg60_millipercent: 1100,
            avg300_millipercent: 500,
        }
    );
    assert_eq!(full, PsiAverages::default());
}

#[test]
fn parses_integer_and_large_values() {
    let raw = "some avg10=0 avg60=12 avg300=1234.5 total=1\n\
               full avg10=123.45 avg60=0.001 avg300=0.00 total=2\n";
    let (some, full) = parse_memory_pressure(raw).expect("parse");
    assert_eq!(some.avg10_millipercent, 0);
    assert_eq!(some.avg60_millipercent, 12_000);
    assert_eq!(some.avg300_millipercent, 1_234_500);
    assert_eq!(full.avg10_millipercent, 123_450);
    assert_eq!(full.avg60_millipercent, 1);
}

#[test]
fn rejects_missing_keys() {
    let raw = "some avg10=0.00 avg60=0.00 total=1\n\
               full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
    assert!(parse_memory_pressure(raw).is_err());
}

#[test]
fn rejects_missing_lines() {
    assert!(parse_memory_pressure("some avg10=0.00 avg60=0.00 avg300=0.00\n").is_err());
}
