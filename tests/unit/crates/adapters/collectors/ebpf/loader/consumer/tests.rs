// Tests for the corresponding production module.
use super::*;

fn collect_batches(raw: Vec<Vec<u8>>, perf_lost: u64) -> Vec<Vec<Vec<u8>>> {
    // Capacity large enough that send_raw_batches never blocks while
    // the test drains afterwards.
    let (tx, rx) = sync_channel::<EventConsumerMessage>(1024);
    assert!(send_raw_batches(&tx, raw, perf_lost, None));
    drop(tx);
    let mut batches = Vec::new();
    while let Ok(EventConsumerMessage::RawBatch { raw, .. }) = rx.recv() {
        batches.push(raw);
    }
    batches
}

#[test]
fn loss_only_cycle_emits_one_empty_batch() {
    let batches = collect_batches(Vec::new(), 7);
    assert_eq!(batches.len(), 1);
    assert!(batches[0].is_empty());
}

#[test]
fn small_batches_are_not_split() {
    let batches = collect_batches(vec![vec![1u8], vec![2u8]], 0);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 2);
}

#[test]
fn batches_split_by_record_count() {
    let raw = (0..MAX_BATCH_RECORDS + 5)
        .map(|i| vec![i as u8; 8])
        .collect::<Vec<_>>();
    let batches = collect_batches(raw, 0);
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].len(), MAX_BATCH_RECORDS);
    assert_eq!(batches[1].len(), 5);
}

#[test]
fn batches_split_by_bytes() {
    let raw = vec![
        vec![0u8; MAX_BATCH_BYTES * 3 / 4],
        vec![1u8; MAX_BATCH_BYTES * 3 / 4],
        vec![2u8; 16],
    ];
    let batches = collect_batches(raw, 0);
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].len(), 1);
    assert_eq!(batches[1].len(), 2);
}

#[test]
fn single_oversized_record_is_not_split() {
    let raw = vec![vec![0u8; MAX_BATCH_BYTES + 1]];
    let batches = collect_batches(raw, 0);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 1);
    assert_eq!(batches[0][0].len(), MAX_BATCH_BYTES + 1);
}
