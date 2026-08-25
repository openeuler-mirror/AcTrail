use std::fmt;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex, RwLock};

use alert_delivery_contract::ForwardAlert;
use arc_swap::{ArcSwap, ArcSwapOption};

use crate::filter::CategoryFilter;
use crate::{
    AlertForwardingConfig, AlertForwardingConfigError, AlertForwardingConfigFileOwner,
    AlertForwardingConfigOwner, AlertForwardingConfigOwnerError,
};

const NO_CONNECTION_GENERATION: u64 = 0;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionGeneration(NonZeroU64);

impl ConnectionGeneration {
    pub const fn new(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone)]
pub struct AlertForwardingPlugin {
    inner: Arc<AlertForwardingPluginInner>,
}

struct AlertForwardingPluginInner {
    owner: Arc<dyn AlertForwardingConfigOwner>,
    requested_config: RwLock<AlertForwardingConfig>,
    filter: ArcSwap<CategoryFilter>,
    active: ArcSwapOption<ActiveConnection>,
    connection_update: Mutex<()>,
    config_update: Mutex<()>,
    requested_enabled: AtomicBool,
    effective_enabled: AtomicBool,
    latest_generation: AtomicU64,
    active_generation: AtomicU64,
    accepted: AtomicU64,
    filtered: AtomicU64,
    dropped: AtomicU64,
    config_persistence_failures: AtomicU64,
}

struct ActiveConnection {
    generation: ConnectionGeneration,
    sender: SyncSender<ForwardingItem>,
}

impl AlertForwardingPlugin {
    pub fn load(path: PathBuf) -> Result<Self, AlertForwardingPluginError> {
        let owner = Arc::new(AlertForwardingConfigFileOwner::new(path)?);
        let config = match owner.load() {
            Ok(config) => config,
            Err(error) if error.is_not_found() => {
                let config = AlertForwardingConfig::disabled();
                owner.persist(&config)?;
                config
            }
            Err(error) => return Err(error.into()),
        };
        Self::new(config, owner)
    }

    pub fn new(
        config: AlertForwardingConfig,
        owner: Arc<dyn AlertForwardingConfigOwner>,
    ) -> Result<Self, AlertForwardingPluginError> {
        config
            .validate()
            .map_err(AlertForwardingPluginError::Config)?;
        Ok(Self {
            inner: Arc::new(AlertForwardingPluginInner {
                owner,
                filter: ArcSwap::from_pointee(CategoryFilter::from_config(&config)),
                requested_enabled: AtomicBool::new(config.enabled()),
                effective_enabled: AtomicBool::new(false),
                requested_config: RwLock::new(config),
                active: ArcSwapOption::empty(),
                connection_update: Mutex::new(()),
                config_update: Mutex::new(()),
                latest_generation: AtomicU64::new(NO_CONNECTION_GENERATION),
                active_generation: AtomicU64::new(NO_CONNECTION_GENERATION),
                accepted: AtomicU64::new(0),
                filtered: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
                config_persistence_failures: AtomicU64::new(0),
            }),
        })
    }

    pub fn next_generation(&self) -> Result<ConnectionGeneration, AlertForwardingPluginError> {
        let previous = self
            .inner
            .latest_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(1)
                    .filter(|next| *next != NO_CONNECTION_GENERATION)
            })
            .map_err(|_| AlertForwardingPluginError::GenerationExhausted)?;
        previous
            .checked_add(1)
            .and_then(ConnectionGeneration::new)
            .ok_or(AlertForwardingPluginError::GenerationExhausted)
    }

    pub fn activate(
        &self,
        generation: ConnectionGeneration,
        sender: SyncSender<ForwardingItem>,
    ) -> bool {
        let _update = self
            .inner
            .connection_update
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.inner.latest_generation.load(Ordering::Acquire) != generation.get() {
            return false;
        }
        self.inner
            .active
            .store(Some(Arc::new(ActiveConnection { generation, sender })));
        self.inner
            .active_generation
            .store(generation.get(), Ordering::Release);
        self.inner.effective_enabled.store(
            self.inner.requested_enabled.load(Ordering::Acquire),
            Ordering::Release,
        );
        true
    }

    pub fn disable_if_generation(&self, generation: ConnectionGeneration) -> bool {
        let _connection_update = self
            .inner
            .connection_update
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(active) = self.inner.active.load_full() else {
            return false;
        };
        if active.generation != generation {
            return false;
        }
        self.inner.effective_enabled.store(false, Ordering::Release);
        self.inner
            .active_generation
            .store(NO_CONNECTION_GENERATION, Ordering::Release);
        self.inner.active.store(None);
        self.disable_requested_config_locked();
        true
    }

    pub fn disable(&self) {
        let _connection_update = self
            .inner
            .connection_update
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.inner.effective_enabled.store(false, Ordering::Release);
        self.inner
            .active_generation
            .store(NO_CONNECTION_GENERATION, Ordering::Release);
        self.inner.active.store(None);
        self.disable_requested_config_locked();
    }

    pub fn deactivate(&self) {
        let _connection_update = self
            .inner
            .connection_update
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.inner.effective_enabled.store(false, Ordering::Release);
        self.inner
            .active_generation
            .store(NO_CONNECTION_GENERATION, Ordering::Release);
        self.inner.active.store(None);
    }

    pub fn is_active_generation(&self, generation: ConnectionGeneration) -> bool {
        self.inner.active_generation.load(Ordering::Acquire) == generation.get()
    }

    #[inline]
    pub fn accepts_category(&self, category: &str) -> bool {
        self.inner.effective_enabled.load(Ordering::Acquire)
            && self.inner.filter.load().accepts(category)
    }

    #[inline]
    pub fn try_publish(&self, alert: ForwardAlert) -> AlertForwardingSubmitOutcome {
        if !self.inner.effective_enabled.load(Ordering::Acquire) {
            return AlertForwardingSubmitOutcome::Disabled;
        }
        let filter = self.inner.filter.load();
        if !filter.accepts(&alert.category) {
            self.inner.filtered.fetch_add(1, Ordering::Relaxed);
            return AlertForwardingSubmitOutcome::Filtered;
        }
        let Some(active) = self.inner.active.load_full() else {
            self.inner.effective_enabled.store(false, Ordering::Release);
            self.inner.dropped.fetch_add(1, Ordering::Relaxed);
            return AlertForwardingSubmitOutcome::Disconnected;
        };
        let generation = active.generation;
        match active.sender.try_send(ForwardingItem { generation, alert }) {
            Ok(()) => {
                self.inner.accepted.fetch_add(1, Ordering::Relaxed);
                AlertForwardingSubmitOutcome::Accepted
            }
            Err(TrySendError::Full(_)) => {
                self.inner.dropped.fetch_add(1, Ordering::Relaxed);
                AlertForwardingSubmitOutcome::QueueFull
            }
            Err(TrySendError::Disconnected(_)) => {
                self.inner.dropped.fetch_add(1, Ordering::Relaxed);
                if self.is_active_generation(generation) {
                    self.inner.effective_enabled.store(false, Ordering::Release);
                }
                AlertForwardingSubmitOutcome::Disconnected
            }
        }
    }

    pub fn config_json(&self) -> Result<String, AlertForwardingPluginError> {
        let effective = self.inner.effective_enabled.load(Ordering::Acquire);
        self.inner
            .requested_config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .with_enabled(effective)
            .to_json()
            .map_err(AlertForwardingPluginError::Config)
    }

    pub fn requested_config_json(&self) -> Result<String, AlertForwardingPluginError> {
        self.inner
            .requested_config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .to_json()
            .map_err(AlertForwardingPluginError::Config)
    }

    pub const fn schema_json() -> &'static str {
        crate::config::ALERT_FORWARDING_CONFIG_SCHEMA
    }

    pub fn validate_config(raw: &str) -> Result<AlertForwardingConfig, AlertForwardingPluginError> {
        AlertForwardingConfig::from_json(raw).map_err(AlertForwardingPluginError::Config)
    }

    pub fn update_config(
        &self,
        raw: &str,
    ) -> Result<AlertForwardingPluginStatus, AlertForwardingPluginError> {
        let _connection_update = self
            .inner
            .connection_update
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _config_update = self
            .inner
            .config_update
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let config = Self::validate_config(raw)?;
        if config.enabled() && self.inner.active.load().is_none() {
            return Err(AlertForwardingPluginError::ProxyDisconnected);
        }
        self.inner.owner.persist(&config)?;
        if !config.enabled() {
            self.inner.effective_enabled.store(false, Ordering::Release);
        }
        self.inner
            .filter
            .store(Arc::new(CategoryFilter::from_config(&config)));
        self.inner
            .requested_enabled
            .store(config.enabled(), Ordering::Release);
        *self
            .inner
            .requested_config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
        let connected = self.inner.active.load().is_some();
        self.inner.effective_enabled.store(
            connected && self.inner.requested_enabled.load(Ordering::Acquire),
            Ordering::Release,
        );
        Ok(self.status())
    }

    pub fn status(&self) -> AlertForwardingPluginStatus {
        let active_generation =
            ConnectionGeneration::new(self.inner.active_generation.load(Ordering::Acquire));
        AlertForwardingPluginStatus {
            requested_enabled: self.inner.requested_enabled.load(Ordering::Acquire),
            effective_enabled: self.inner.effective_enabled.load(Ordering::Acquire),
            active_generation,
            accepted: self.inner.accepted.load(Ordering::Relaxed),
            filtered: self.inner.filtered.load(Ordering::Relaxed),
            dropped: self.inner.dropped.load(Ordering::Relaxed),
            config_persistence_failures: self
                .inner
                .config_persistence_failures
                .load(Ordering::Relaxed),
        }
    }

    pub fn record_delivery_drop(&self) {
        self.inner.dropped.fetch_add(1, Ordering::Relaxed);
    }

    fn disable_requested_config_locked(&self) {
        let _config_update = self
            .inner
            .config_update
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.inner.requested_enabled.store(false, Ordering::Release);
        let config = self
            .inner
            .requested_config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .with_enabled(false);
        *self
            .inner
            .requested_config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = config.clone();
        if self.inner.owner.persist(&config).is_err() {
            self.inner
                .config_persistence_failures
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[derive(Debug)]
pub struct ForwardingItem {
    generation: ConnectionGeneration,
    alert: ForwardAlert,
}

impl ForwardingItem {
    pub const fn generation(&self) -> ConnectionGeneration {
        self.generation
    }

    pub fn alert(&self) -> &ForwardAlert {
        &self.alert
    }

    pub fn into_alert(self) -> ForwardAlert {
        self.alert
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlertForwardingPluginStatus {
    pub requested_enabled: bool,
    pub effective_enabled: bool,
    pub active_generation: Option<ConnectionGeneration>,
    pub accepted: u64,
    pub filtered: u64,
    pub dropped: u64,
    pub config_persistence_failures: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertForwardingSubmitOutcome {
    Accepted,
    Disabled,
    Filtered,
    QueueFull,
    Disconnected,
}

#[derive(Debug)]
pub enum AlertForwardingPluginError {
    Config(AlertForwardingConfigError),
    ConfigOwner(AlertForwardingConfigOwnerError),
    ProxyDisconnected,
    GenerationExhausted,
}

impl fmt::Display for AlertForwardingPluginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(formatter, "{error}"),
            Self::ConfigOwner(error) => write!(formatter, "{error}"),
            Self::ProxyDisconnected => formatter
                .write_str("alert forwarding cannot be enabled without an active proxy connection"),
            Self::GenerationExhausted => {
                formatter.write_str("alert forwarding connection generation is exhausted")
            }
        }
    }
}

impl std::error::Error for AlertForwardingPluginError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::ConfigOwner(error) => Some(error),
            Self::ProxyDisconnected | Self::GenerationExhausted => None,
        }
    }
}

impl From<AlertForwardingConfigOwnerError> for AlertForwardingPluginError {
    fn from(error: AlertForwardingConfigOwnerError) -> Self {
        Self::ConfigOwner(error)
    }
}
