use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::sync::mpsc::TrySendError;
use std::sync::Arc;

use sandbox_plugin_delivery::{
    SandboxConsumerBatch, SandboxConsumerDelivery, SandboxConsumerId, SandboxDeliveryOutcome,
    SandboxObservationKind, SandboxPluginPublisher, SandboxPublishBatch, SandboxPublishError,
    SandboxPublishReport, SandboxRoutePlan,
};

use super::registry::{ConsumerEndpoint, SandboxPluginFacade};

impl SandboxPluginPublisher for SandboxPluginFacade {
    fn publish(
        &self,
        batch: SandboxPublishBatch,
        plan: SandboxRoutePlan,
    ) -> Result<SandboxPublishReport, SandboxPublishError> {
        let snapshot = self.registry.snapshot.load();
        if plan.generation() != snapshot.generation {
            return Err(SandboxPublishError::ExpiredPlan {
                plan_generation: plan.generation(),
                current_generation: snapshot.generation,
            });
        }
        let actual = u32::try_from(batch.observations().len())
            .map_err(|_| SandboxPublishError::ObservationCountOverflow)?;
        if plan.observation_count() != actual {
            return Err(SandboxPublishError::ObservationCountMismatch {
                planned: plan.observation_count(),
                actual,
            });
        }
        validate_plan(&plan, &batch, &snapshot.endpoints)?;
        let (_, _, routes, _) = plan.into_parts();
        let mut deliveries = Vec::with_capacity(routes.len());
        for route in routes.into_vec() {
            let (consumer_id, indices) = route.into_parts();
            let endpoint = snapshot
                .endpoints
                .get(&consumer_id)
                .expect("validated sandbox consumer route");
            let observation_count = u32::try_from(indices.len()).unwrap_or(u32::MAX);
            let consumer_batch = SandboxConsumerBatch::new(
                batch.source(),
                batch.sequence(),
                Arc::clone(batch.observations()),
                Arc::from(indices),
            );
            let outcome = endpoint.try_publish(consumer_batch, observation_count);
            deliveries.push(SandboxConsumerDelivery {
                consumer_id,
                outcome,
            });
        }
        Ok(SandboxPublishReport { deliveries })
    }
}

impl ConsumerEndpoint {
    fn try_publish(
        &self,
        batch: SandboxConsumerBatch,
        observation_count: u32,
    ) -> SandboxDeliveryOutcome {
        let sender = self.sender();
        self.metrics().queue_depth.fetch_add(1, Ordering::Relaxed);
        match sender.try_send(batch) {
            Ok(()) => SandboxDeliveryOutcome::Accepted { observation_count },
            Err(TrySendError::Full(_)) => {
                self.metrics().queue_depth.fetch_sub(1, Ordering::Relaxed);
                self.metrics()
                    .dropped_records
                    .fetch_add(u64::from(observation_count), Ordering::Relaxed);
                SandboxDeliveryOutcome::Full { observation_count }
            }
            Err(TrySendError::Disconnected(_)) => {
                self.metrics().queue_depth.fetch_sub(1, Ordering::Relaxed);
                self.metrics().closed.store(true, Ordering::Relaxed);
                self.metrics()
                    .dropped_records
                    .fetch_add(u64::from(observation_count), Ordering::Relaxed);
                SandboxDeliveryOutcome::Closed { observation_count }
            }
        }
    }
}

fn validate_plan(
    plan: &SandboxRoutePlan,
    batch: &SandboxPublishBatch,
    endpoints: &std::collections::BTreeMap<SandboxConsumerId, Arc<ConsumerEndpoint>>,
) -> Result<(), SandboxPublishError> {
    let observations = batch.observations();
    let mut routed = vec![false; observations.len()];
    let mut route_seen = vec![0_usize; observations.len()];
    let mut consumers = BTreeSet::new();
    if plan.routes().is_empty() {
        return Err(SandboxPublishError::EmptyRoutePlan);
    }
    for (route_offset, route) in plan.routes().iter().enumerate() {
        let consumer_id = route.consumer_id();
        if !consumers.insert(consumer_id) {
            return Err(SandboxPublishError::DuplicateConsumerRoute { consumer_id });
        }
        let Some(endpoint) = endpoints.get(&consumer_id) else {
            return Err(SandboxPublishError::MissingConsumer { consumer_id });
        };
        if route.observation_indices().is_empty() {
            return Err(SandboxPublishError::EmptyConsumerRoute { consumer_id });
        }
        let route_marker = route_offset + 1;
        for index in route.observation_indices() {
            let Some(observation) = usize::try_from(*index)
                .ok()
                .and_then(|index| observations.get(index))
            else {
                return Err(SandboxPublishError::InvalidObservationIndex {
                    consumer_id,
                    index: *index,
                });
            };
            if route_seen[*index as usize] == route_marker {
                return Err(SandboxPublishError::DuplicateObservationIndex {
                    consumer_id,
                    index: *index,
                });
            }
            route_seen[*index as usize] = route_marker;
            if !endpoint.selector_matches(SandboxObservationKind::of(observation)) {
                return Err(SandboxPublishError::SelectorMismatch {
                    consumer_id,
                    index: *index,
                });
            }
            routed[*index as usize] = true;
        }
    }
    let mut unmatched = vec![false; observations.len()];
    for index in plan.unmatched_indices() {
        let Some(slot) = usize::try_from(*index)
            .ok()
            .and_then(|index| unmatched.get_mut(index))
        else {
            return Err(SandboxPublishError::InvalidUnmatchedIndex { index: *index });
        };
        if *slot {
            return Err(SandboxPublishError::DuplicateUnmatchedIndex { index: *index });
        }
        if routed[*index as usize] {
            return Err(SandboxPublishError::RoutedAndUnmatched { index: *index });
        }
        *slot = true;
    }
    for (index, assigned) in routed
        .iter()
        .zip(unmatched.iter())
        .map(|(routed, unmatched)| *routed || *unmatched)
        .enumerate()
    {
        if !assigned {
            return Err(SandboxPublishError::UnassignedObservation {
                index: index as u32,
            });
        }
    }
    Ok(())
}
