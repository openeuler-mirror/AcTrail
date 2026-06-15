use crate::{
    BestEffortDelivery, BestEffortDeliveryConfig, BestEffortSink, ExportError, ExportPublishResult,
    SemanticActionExportAdapter, SemanticActionExportRecord,
};

pub trait SemanticActionExportRoute {
    fn name(&self) -> &'static str;

    fn publish(
        &self,
        record: SemanticActionExportRecord<'_>,
    ) -> Result<ExportPublishResult, ExportError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BestEffortSemanticActionRouteConfig {
    pub worker_thread_name: &'static str,
    pub queue_capacity: u32,
}

pub struct BestEffortSemanticActionRoute<A>
where
    A: SemanticActionExportAdapter,
{
    adapter: A,
    delivery: BestEffortDelivery<A::Message>,
}

impl<A> BestEffortSemanticActionRoute<A>
where
    A: SemanticActionExportAdapter,
{
    pub fn spawn<S>(
        adapter: A,
        config: BestEffortSemanticActionRouteConfig,
        sink: S,
    ) -> Result<Self, ExportError>
    where
        S: BestEffortSink<A::Message>,
    {
        let delivery = BestEffortDelivery::spawn(
            BestEffortDeliveryConfig {
                component_name: adapter.name(),
                worker_thread_name: config.worker_thread_name,
                queue_capacity: config.queue_capacity,
            },
            sink,
        )?;
        Ok(Self { adapter, delivery })
    }
}

impl<A> SemanticActionExportRoute for BestEffortSemanticActionRoute<A>
where
    A: SemanticActionExportAdapter,
{
    fn name(&self) -> &'static str {
        self.adapter.name()
    }

    fn publish(
        &self,
        record: SemanticActionExportRecord<'_>,
    ) -> Result<ExportPublishResult, ExportError> {
        self.delivery.check_health()?;
        let message = self.adapter.encode(record)?;
        self.delivery.publish(message)
    }
}
