//! Background TLS plan lookups and replies.

use super::*;

impl TlsSyncPlanWorker {
    pub(super) fn run(mut self, receiver: Receiver<WorkerRequest>) {
        for request in receiver {
            let mut job = match request {
                WorkerRequest::Lookup(job) => job,
                WorkerRequest::Direct(object) => {
                    if let Some(worker) = &mut self.direct {
                        worker.resolve(object, &self.analysis_cache, self.match_limit);
                    }
                    continue;
                }
            };
            if job
                .response
                .as_ref()
                .is_some_and(|response| peer_closed(response))
            {
                continue;
            }
            if job.consumer == ProbeConsumer::Direct {
                let reply = self.lookup_direct(&job.runtime_binary, job.peer_root);
                if let Some(sender) = job.control_response {
                    let _ = sender.send(LaunchPlanLookupOutcome { reply });
                }
                continue;
            }
            let outcome = self.lookup(
                &job.runtime_binary,
                job.consumer,
                job.peer_root,
                job.pin_peer_path,
            );
            let Some(response_stream) = job.response.as_mut() else {
                if let Some(sender) = job.control_response {
                    let _ = sender.send(LaunchPlanLookupOutcome {
                        reply: Ok(launch_reply_for_outcome(outcome)),
                    });
                }
                continue;
            };
            if let Err(error) = tls_payload_sync::FrameCodec::write_lookup_response(
                response_stream,
                &outcome.response,
                self.config.sync_max_frame_bytes as usize,
            ) {
                tracing::warn!(
                    target: "actrail::tls_sync",
                    binary = %job.runtime_binary.display(),
                    error = %error,
                    "failed to write TLS sync plan lookup response"
                );
            }
        }
    }

    fn lookup_direct(
        &mut self,
        runtime_binary: &Path,
        peer_root: Option<Result<PeerRootHandle, String>>,
    ) -> Result<LaunchTlsPlanReply, ControlError> {
        let started = Instant::now();
        let root = peer_root
            .ok_or_else(|| ControlError::new("tls_sync_plan_root", ""))?
            .map_err(|error| ControlError::new("tls_sync_plan_root", error))?;
        let pinned = root
            .pin_path(runtime_binary)
            .map_err(|error| ControlError::new("tls_sync_plan_root", error))?;
        let startup_object = DirectObject::open_startup(&pinned.path());
        let before = self.analysis_cache.stats();
        let probe_binary = pinned.path();
        let resolved = self.resolve_plans(
            &probe_binary,
            runtime_binary,
            ProbeConsumer::Direct,
            Some(root.into_file_source(runtime_binary, Some(pinned))),
        );
        if let (Some(worker), Some(object), Ok(plans)) = (&self.direct, startup_object, &resolved) {
            // A successful Auto/All pass completed the executable branches.
            // Only the pinned root's facts belong in its single-object cache entry.
            let root_plans = plans.iter().filter(|plan| {
                plan.source == "executable"
                    && plan.target == runtime_binary
                    && plan.binary == runtime_binary
            });
            if root_plans
                .clone()
                .all(|plan| matches!(plan.provider.as_str(), "openssl" | "rustls"))
            {
                worker.seed_startup(
                    object,
                    root_plans.map(|plan| {
                        (
                            plan.binary_identity.clone(),
                            plan.provider.clone(),
                            plan.points.clone(),
                        )
                    }),
                );
            }
        }
        let status = match resolved {
            Ok(plans) if plans.is_empty() => LaunchTlsPlanStatus::Unsupported {
                reason: LaunchTlsPlanUnavailableReason::NoProbePoints,
            },
            Ok(plans) => LaunchTlsPlanStatus::Found(
                plans
                    .into_iter()
                    .map(|plan| LaunchTlsPlanDescriptor {
                        target: plan.target,
                        binary: plan.binary,
                        target_identity: plan.target_identity,
                        binary_identity: plan.binary_identity,
                        provider: plan.provider,
                        source: plan.source,
                        points: plan.points,
                    })
                    .collect(),
            ),
            Err(_) => LaunchTlsPlanStatus::Unsupported {
                reason: LaunchTlsPlanUnavailableReason::AnalysisRejected,
            },
        };
        let after = self.analysis_cache.stats();
        Ok(LaunchTlsPlanReply {
            status,
            cache_hit: after.misses == before.misses && after.hits > before.hits,
            resolve_elapsed_micros: duration_micros(started.elapsed()),
        })
    }

    fn lookup(
        &mut self,
        runtime_binary: &Path,
        consumer: ProbeConsumer,
        peer_root: Option<Result<PeerRootHandle, String>>,
        pin_peer_path: bool,
    ) -> PlanLookupOutcome {
        let started = Instant::now();
        let peer_root = match peer_root {
            Some(Ok(root)) => Some(root),
            Some(Err(reason)) => {
                tracing::warn!(
                    target: "actrail::tls_sync",
                    runtime_binary = %runtime_binary.display(),
                    reason = %reason,
                    "TLS sync plan lookup path resolution failed"
                );
                return unsupported_outcome(reason, started);
            }
            None => None,
        };
        let pinned_probe = if pin_peer_path {
            match peer_root.as_ref() {
                Some(root) => match root.pin_path(runtime_binary) {
                    Ok(path) => Some(path),
                    Err(reason) => {
                        tracing::warn!(
                            target: "actrail::tls_sync",
                            runtime_binary = %runtime_binary.display(),
                            reason = %reason,
                            "TLS sync plan lookup path resolution failed"
                        );
                        return unsupported_outcome(reason, started);
                    }
                },
                None => {
                    return unsupported_outcome(
                        "authenticated peer root is required for pinned TLS plan lookup"
                            .to_string(),
                        started,
                    );
                }
            }
        } else {
            None
        };
        let probe_binary = match pinned_probe.as_ref() {
            Some(path) => path.path(),
            None => runtime_binary.to_path_buf(),
        };
        let cache_before = self.analysis_cache.stats();
        let record = match self.resolve_plans(
            &probe_binary,
            runtime_binary,
            consumer,
            peer_root.map(|root| root.into_file_source(runtime_binary, pinned_probe)),
        ) {
            Ok(plans) => BinaryPlanRecord { plans },
            Err(error) => {
                tracing::warn!(
                    target: "actrail::tls_sync",
                    runtime_binary = %runtime_binary.display(),
                    probe_binary = %probe_binary.display(),
                    error = %error.message,
                    "TLS sync plan lookup probe failed"
                );
                return unsupported_outcome(error.message, started);
            }
        };
        let cache_after = self.analysis_cache.stats();
        let cache_hit =
            cache_after.misses == cache_before.misses && cache_after.hits > cache_before.hits;
        self.log_cache_lookup(
            consumer,
            runtime_binary,
            &probe_binary,
            cache_hit,
            cache_after,
        );
        outcome_for_record(record, cache_hit, started)
    }

    fn resolve_plans(
        &self,
        probe_binary: &Path,
        runtime_binary: &Path,
        consumer: ProbeConsumer,
        source: Option<Rc<dyn ProbeFileSource>>,
    ) -> Result<Vec<BinaryPlanDescriptor>, ControlError> {
        let request = FastProbeRequest {
            binary: if source.is_some() {
                runtime_binary
            } else {
                probe_binary
            }
            .to_path_buf(),
            arch: ArchFilter::Auto,
            provider: ProviderFilter::Auto,
            source: SourceFilter::Auto,
            match_limit: self.match_limit,
            libraries: library_candidates(&self.config),
            library_search_dirs: Vec::new(),
        };
        let resolution = match source {
            Some(source) => resolve_plans_with_file_source(
                request,
                consumer,
                ResolveMode::All,
                Rc::clone(&self.analysis_cache),
                source,
            ),
            None => resolve_plans_with_analysis_cache(
                request,
                consumer,
                ResolveMode::All,
                Rc::clone(&self.analysis_cache),
            ),
        }
        .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?;
        if resolution.plans.is_empty() && consumer != ProbeConsumer::Direct {
            return Err(ControlError::new(
                "tls_sync_plan",
                "no supported TLS payload probe points found",
            ));
        }
        resolution
            .plans
            .into_iter()
            .map(|plan| {
                // Daemon discovery already validates the actual backend selected by
                // the target ELF. Static targets use uprobes, whose lifecycle points
                // are not inline hooks. Each installer revalidates binary identity.
                if consumer == ProbeConsumer::Sync {
                    validate_native_backend_plan(&plan)
                        .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?;
                }
                Ok(BinaryPlanDescriptor {
                    target: runtime_view_binary(&plan.target.binary, runtime_binary, probe_binary),
                    binary: runtime_view_binary(&plan.binary.path, runtime_binary, probe_binary),
                    target_identity: plan.target.identity.clone(),
                    binary_identity: plan.binary.identity.clone(),
                    provider: plan.provider.as_str().to_string(),
                    source: plan.source.as_str().to_string(),
                    points: encode_points(&plan)
                        .map_err(|error| ControlError::new("tls_sync_plan", error.to_string()))?,
                })
            })
            .collect()
    }

    fn log_cache_lookup(
        &self,
        consumer: ProbeConsumer,
        runtime_binary: &Path,
        probe_binary: &Path,
        cache_hit: bool,
        stats: BinaryAnalysisCacheStats,
    ) {
        if !self.config.diagnostics_enabled {
            return;
        }
        tracing::info!(
            target: "actrail::tls_sync",
            consumer = probe_consumer_name(consumer),
            runtime_binary = %runtime_binary.display(),
            probe_binary = %probe_binary.display(),
            cache = if cache_hit { "hit" } else { "miss" },
            cache_entries = stats.entries,
            cache_evictions = stats.evictions,
            "TLS binary analysis cache lookup"
        );
    }
}

fn peer_closed(stream: &std::os::unix::net::UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut descriptor = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLRDHUP,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
        if result >= 0 {
            return descriptor.revents
                & (libc::POLLRDHUP | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL)
                != 0;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return true;
        }
    }
}
