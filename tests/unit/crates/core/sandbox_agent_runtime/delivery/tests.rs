// Tests for the corresponding production module.
use super::*;

#[test]
fn workload_publication_requires_negotiated_capability() {
    let gate = ConnectionGate::disconnected();
    let generation = ConnectionGeneration::new(7).unwrap();

    gate.enable(generation, false);
    assert_eq!(gate.generation(), Some(generation));
    assert_eq!(gate.workload_generation(), None);

    gate.enable(generation, true);
    assert_eq!(gate.workload_generation(), Some(generation));

    gate.disable();
    assert_eq!(gate.generation(), None);
    assert_eq!(gate.workload_generation(), None);
}
