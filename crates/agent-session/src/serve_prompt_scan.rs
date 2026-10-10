//! Provider transcript discovery for the last-prompt projection. A discovery
//! scan walks the provider history (Codex `sessions`, Claude `projects`) and can
//! take seconds on a large history; failed scans retry with a capped backoff.
use super::*;

/// Why no transcript source is available yet.
enum NoSource {
    Scanning,
    Unavailable,
}

impl ProviderPromptDiscoveryRegistry {
    /// Resolves the transcript source, waiting for a scan when one is due.
    pub(super) async fn resolve_source(
        &self,
        record: &SessionRecord,
    ) -> Option<ProviderPromptSource> {
        self.resolve(record, true).await.ok()
    }

    /// The session list never waits for a scan: it starts or shares one and
    /// reports the prompt pending, and a later pass reads the result. A slot
    /// whose source was invalidated, a scan backoff, or a registry-bound refusal
    /// reports the prompt unavailable.
    pub(super) async fn cached_source_or_start_scan(
        &self,
        record: &SessionRecord,
        key: &ProviderPromptDiscoveryKey,
    ) -> Result<ProviderPromptSource, LastPromptProjection> {
        let scanning = match self.resolve(record, false).await {
            Ok(source) => return Ok(source),
            Err(NoSource::Scanning) => true,
            Err(NoSource::Unavailable) => false,
        };
        let slot = self.entries.lock().await.get(key).cloned();
        let (invalidated, continuity) = match slot {
            Some(slot) => {
                let state = slot.lock().await;
                let continuity = state.last_prompt_continuity.clone();
                (state.last_prompt_invalidated, Some(continuity))
            }
            None => (true, None),
        };
        Err(LastPromptProjection {
            state: if scanning && !invalidated {
                LastPromptState::Pending
            } else {
                LastPromptState::Unavailable
            },
            continuity,
            prompt: None,
        })
    }

    async fn resolve(
        &self,
        record: &SessionRecord,
        wait: bool,
    ) -> Result<ProviderPromptSource, NoSource> {
        let key = ProviderPromptDiscoveryKey::from_record(record).ok_or(NoSource::Unavailable)?;
        let slot = {
            let mut entries = self.entries.lock().await;
            entries.retain(|existing, _| existing.session_id != key.session_id || existing == &key);
            if !entries.contains_key(&key) && entries.len() >= PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES
            {
                // Stable admission avoids turning every list pass above the
                // registry bound into another expensive cold-recovery scan.
                return Err(NoSource::Unavailable);
            }
            entries
                .entry(key.clone())
                .or_insert_with(|| {
                    Arc::new(tokio::sync::Mutex::new(
                        ProviderPromptDiscoverySlot::default(),
                    ))
                })
                .clone()
        };
        loop {
            let mut state = slot.lock().await;
            if let Some(source) = state.source.clone() {
                return Ok(source);
            }
            let now = Instant::now();
            if state.next_scan_at.is_some_and(|next| now < next) {
                return Err(NoSource::Unavailable);
            }
            let mut progress = state.progress.subscribe();
            if state.in_flight {
                if !wait {
                    return Err(NoSource::Scanning);
                }
                drop(state);
                let _ = progress.changed().await;
                continue;
            }
            state.in_flight = true;
            state.scan_attempts = state.scan_attempts.saturating_add(1);
            drop(state);

            let resolver = self.resolver.clone();
            let scan_permits = self.scan_permits.clone();
            let candidate = record.clone();
            let task_slot = slot.clone();
            tokio::spawn(async move {
                let source = match scan_permits.acquire_owned().await {
                    Ok(_permit) => tokio::task::spawn_blocking(move || resolver(&candidate))
                        .await
                        .ok()
                        .flatten(),
                    Err(_) => None,
                };
                let mut state = task_slot.lock().await;
                state.in_flight = false;
                if let Some(source) = source {
                    state.source = Some(source);
                    state.next_scan_at = None;
                } else {
                    state.next_scan_at = Some(Instant::now() + state.backoff);
                    state.backoff = next_provider_prompt_discovery_backoff(state.backoff);
                }
                state.progress.send_modify(|version| {
                    *version = version.wrapping_add(1);
                });
            });
            if !wait {
                return Err(NoSource::Scanning);
            }
            let _ = progress.changed().await;
        }
    }
}

pub(super) fn next_provider_prompt_discovery_backoff(current: Duration) -> Duration {
    current
        .saturating_mul(2)
        .min(PROVIDER_PROMPT_DISCOVERY_MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::super::tests::provider_discovery_record;
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn provider_prompt_discovery_backoff_doubles_and_caps() {
        let mut backoff = PROVIDER_PROMPT_PENDING_POLL_INTERVAL;
        let mut observed = Vec::new();
        for _ in 0..8 {
            backoff = next_provider_prompt_discovery_backoff(backoff);
            observed.push(backoff);
        }
        assert_eq!(observed[0], Duration::from_secs(1));
        assert_eq!(observed[1], Duration::from_secs(2));
        assert_eq!(observed[2], Duration::from_secs(4));
        assert_eq!(observed[5], PROVIDER_PROMPT_DISCOVERY_MAX_BACKOFF);
        assert_eq!(observed[7], PROVIDER_PROMPT_DISCOVERY_MAX_BACKOFF);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn last_prompt_projection_does_not_wait_for_provider_history_scan() {
        let release = Arc::new(AtomicBool::new(false));
        let registry = Arc::new(ProviderPromptDiscoveryRegistry::with_resolver({
            let release = release.clone();
            move |record| {
                while !release.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Some(ProviderPromptSource::test_path(
                    ProviderKind::Codex,
                    record.id.clone(),
                    PathBuf::from("/nonexistent/slow-scan.jsonl"),
                ))
            }
        }));
        // A failed assertion must still release the blocked scans, or runtime
        // shutdown waits on their blocking threads forever.
        struct ReleaseOnDrop(Arc<AtomicBool>);
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let _release_on_drop = ReleaseOnDrop(release.clone());
        let list_pass = |record: SessionRecord| {
            let registry = registry.clone();
            async move {
                tokio::time::timeout(
                    Duration::from_millis(500),
                    registry.last_prompt_projection(&record),
                )
                .await
                .expect("GET /sessions must not wait for a provider-history scan")
                .expect("projection")
            }
        };
        let record =
            provider_discovery_record("slow-history-scan", "hs-slow-scan", "launch-slow-scan", 1);
        let key = ProviderPromptDiscoveryKey::from_record(&record).expect("discovery key");

        for _ in 0..2 {
            let projection = list_pass(record.clone()).await;
            assert_eq!(
                projection.state,
                LastPromptState::Pending,
                "exact-source discovery in progress is pending"
            );
            assert_eq!(projection.prompt, None);
            assert_eq!(
                projection.continuity,
                registry.continuity_for_key(&key).await
            );
        }
        assert_eq!(
            registry.scan_attempts(&record).await,
            1,
            "list passes during a scan must share it, not start another"
        );

        let bound: Vec<_> = (0..PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES)
            .map(|index| {
                provider_discovery_record(
                    &format!("bound-{index}"),
                    &format!("hs-bound-{index}"),
                    "launch-bound",
                    1,
                )
            })
            .collect();
        for record in &bound[..PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES - 1] {
            list_pass(record.clone()).await;
        }
        let refused = list_pass(bound[PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES - 1].clone()).await;
        assert_eq!(
            refused.state,
            LastPromptState::Unavailable,
            "a registry-bound refusal is unavailable, not pending"
        );

        release.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(5), async {
            while registry.in_flight_count().await > 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("background scans complete");
        assert!(
            registry.resolve_source(&record).await.is_some(),
            "the background scan result serves the next pass"
        );
        assert_eq!(registry.scan_attempts(&record).await, 1);

        registry.invalidate_source(&record).await;
        assert_eq!(
            list_pass(record.clone()).await.state,
            LastPromptState::Unavailable,
            "rediscovery after an invalidated source stays unavailable"
        );
    }
}
