use std::collections::BTreeMap;

use sandbox_plugin_delivery::{
    SandboxConsumerRoute, SandboxIntentQueryError, SandboxIntentQueryResult,
    SandboxObservationDescriptor, SandboxPluginIntentMatcher, SandboxRoutePlan,
};

use super::registry::SandboxPluginFacade;

impl SandboxPluginIntentMatcher for SandboxPluginFacade {
    fn query_intent(
        &self,
        descriptors: &[SandboxObservationDescriptor],
    ) -> Result<SandboxIntentQueryResult, SandboxIntentQueryError> {
        let observation_count = u32::try_from(descriptors.len())
            .map_err(|_| SandboxIntentQueryError::ObservationCountOverflow)?;
        validate_descriptors(descriptors, observation_count)?;
        let snapshot = self.registry.snapshot.load();
        let mut route_indices = BTreeMap::new();
        let mut unmatched = Vec::new();
        for descriptor in descriptors {
            let consumers = snapshot.consumers_for(descriptor.kind());
            if consumers.is_empty() {
                unmatched.push(descriptor.observation_index());
                continue;
            }
            for consumer_id in consumers {
                route_indices
                    .entry(*consumer_id)
                    .or_insert_with(Vec::new)
                    .push(descriptor.observation_index());
            }
        }
        if route_indices.is_empty() {
            return Ok(SandboxIntentQueryResult::NoInterest {
                generation: snapshot.generation,
                observation_count,
            });
        }
        let routes = route_indices
            .into_iter()
            .map(|(consumer_id, indices)| {
                SandboxConsumerRoute::new(consumer_id, indices.into_boxed_slice())
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(SandboxIntentQueryResult::Matched(SandboxRoutePlan::new(
            snapshot.generation,
            observation_count,
            routes,
            unmatched.into_boxed_slice(),
        )))
    }
}

fn validate_descriptors(
    descriptors: &[SandboxObservationDescriptor],
    observation_count: u32,
) -> Result<(), SandboxIntentQueryError> {
    let mut seen = vec![false; descriptors.len()];
    for descriptor in descriptors {
        let index = descriptor.observation_index();
        if index >= observation_count {
            return Err(SandboxIntentQueryError::InvalidDescriptorIndex { index });
        }
        let slot = &mut seen[index as usize];
        if *slot {
            return Err(SandboxIntentQueryError::DuplicateDescriptorIndex { index });
        }
        *slot = true;
    }
    Ok(())
}
