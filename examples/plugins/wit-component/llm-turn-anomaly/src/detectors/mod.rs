mod consecutive_retry;
mod context_growth;
mod error_ratio;
mod high_frequency;
mod repeated_similar;

use alloc::string::String;

use consecutive_retry::ConsecutiveRetryDetector;
use context_growth::ContextGrowthDetector;
use error_ratio::ErrorRatioDetector;
use high_frequency::HighFrequencyDetector;
use repeated_similar::RepeatedSimilarDetector;

use crate::LlmTurnAnomalyConfig;
use crate::actrail::plugin::types::{
    LlmExchangeRecord, LlmResponseStatus, TraceActivityContext,
};

pub(crate) struct ResponseOutcome {
    pub(crate) request_action_id: String,
    pub(crate) process_id: String,
    pub(crate) model: Option<String>,
    pub(crate) request_body_bytes: u64,
    pub(crate) started_at: u64,
    pub(crate) status: LlmResponseStatus,
}

impl ResponseOutcome {
    pub(crate) fn from_exchange(exchange: &LlmExchangeRecord) -> Option<Self> {
        if exchange.response_status == LlmResponseStatus::Pending {
            return None;
        }
        Some(Self {
            request_action_id: exchange.request_action_id.clone(),
            process_id: exchange.process_id.clone(),
            model: exchange.model.clone(),
            request_body_bytes: exchange.request_body_bytes,
            started_at: exchange.started_at,
            status: exchange.response_status,
        })
    }
}

#[derive(Default)]
pub(crate) struct DetectorState {
    high_frequency: HighFrequencyDetector,
    consecutive_retry: ConsecutiveRetryDetector,
    repeated_similar: RepeatedSimilarDetector,
    error_ratio: ErrorRatioDetector,
    context_growth: ContextGrowthDetector,
}

impl DetectorState {
    pub(crate) fn observe(
        &mut self,
        config: &LlmTurnAnomalyConfig,
        exchange: &LlmExchangeRecord,
    ) -> Result<(), String> {
        let group = ExchangeGroup {
            process_id: exchange.process_id.clone(),
            model: exchange.model.clone(),
        };

        if config.high_frequency.enabled {
            self.high_frequency.observe(config, &group, exchange)?;
        }
        if config.repeated_similar.enabled {
            self.repeated_similar.observe(config, &group, exchange)?;
        }
        if config.context_growth.enabled {
            self.context_growth.observe(config, &group, exchange)?;
        }
        Ok(())
    }

    pub(crate) fn observe_response(
        &mut self,
        config: &LlmTurnAnomalyConfig,
        outcome: &ResponseOutcome,
    ) -> Result<(), String> {
        let group = ExchangeGroup {
            process_id: outcome.process_id.clone(),
            model: outcome.model.clone(),
        };
        if config.consecutive_retry.enabled {
            self.consecutive_retry
                .observe(config, &group, outcome)?;
        }
        if config.error_ratio.enabled {
            self.error_ratio.observe(config, &group, outcome)?;
        }
        Ok(())
    }

    pub(crate) fn evaluate(
        &mut self,
        trace_id: &str,
        alert_token: &[u8],
        context: &TraceActivityContext,
        config: &LlmTurnAnomalyConfig,
    ) -> Result<(), String> {
        if config.high_frequency.enabled {
            self.high_frequency
                .evaluate(trace_id, alert_token, context, config)?;
        }
        if config.consecutive_retry.enabled {
            self.consecutive_retry
                .evaluate(trace_id, alert_token, context, config)?;
        }
        if config.repeated_similar.enabled {
            self.repeated_similar
                .evaluate(trace_id, alert_token, context, config)?;
        }
        if config.error_ratio.enabled {
            self.error_ratio
                .evaluate(trace_id, alert_token, context, config)?;
        }
        if config.context_growth.enabled {
            self.context_growth
                .evaluate(trace_id, alert_token, context, config)?;
        }
        Ok(())
    }
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct ExchangeGroup {
    process_id: String,
    model: Option<String>,
}
