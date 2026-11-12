//! Bounded incremental prompt and message projection for session lists.
use super::*;

pub(in crate::serve) type ProviderLastPromptTrackerOpener = dyn Fn(ProviderPromptSource, Option<(u64, String, String)>) -> Option<ProviderLastPromptTracker>
    + Send
    + Sync;

impl ProviderPromptDiscoveryRegistry {
    #[cfg(test)]
    pub(in crate::serve) fn with_resolver_and_tracker_opener<F, G>(
        resolver: F,
        tracker_opener: G,
    ) -> Self
    where
        F: Fn(&crate::SessionRecord) -> Option<ProviderPromptSource> + Send + Sync + 'static,
        G: Fn(
                ProviderPromptSource,
                Option<(u64, String, String)>,
            ) -> Option<ProviderLastPromptTracker>
            + Send
            + Sync
            + 'static,
    {
        Self {
            entries: tokio::sync::Mutex::new(HashMap::new()),
            resolver: Arc::new(resolver),
            scan_permits: Arc::new(tokio::sync::Semaphore::new(
                PROVIDER_PROMPT_DISCOVERY_MAX_CONCURRENT_SCANS,
            )),
            tracker_opener: Arc::new(tracker_opener),
            recovery_permits: Arc::new(tokio::sync::Semaphore::new(
                PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS,
            )),
        }
    }

    pub(in crate::serve) async fn last_prompt_projection(
        &self,
        record: &crate::SessionRecord,
        state_dir: Option<&std::path::Path>,
    ) -> Option<LastPromptProjection> {
        let key = ProviderPromptDiscoveryKey::from_record(record)?;
        let source = match self.cached_source_or_start_scan(record, &key).await {
            Ok(source) => source,
            Err(projection) => return Some(*projection),
        };
        let Some(slot) = self.entries.lock().await.get(&key).cloned() else {
            return Some(LastPromptProjection {
                state: LastPromptState::Unavailable,
                continuity: None,
                prompt: None,
                message: None,
            });
        };
        let (tracker_cell, unavailable_until_current, continuity) = {
            let mut state = slot.lock().await;
            if state.last_prompt_in_flight {
                return Some(LastPromptProjection {
                    state: if state.last_prompt_invalidated {
                        LastPromptState::Unavailable
                    } else {
                        LastPromptState::Pending
                    },
                    continuity: Some(state.last_prompt_continuity.clone()),
                    prompt: None,
                    message: None,
                });
            }
            state.last_prompt_in_flight = true;
            (
                state.last_prompt_tracker.clone(),
                state.last_prompt_invalidated,
                state.last_prompt_continuity.clone(),
            )
        };
        let tracker_opener = self.tracker_opener.clone();
        let recovery_permits = self.recovery_permits.clone();
        let state_dir = state_dir.map(std::path::Path::to_path_buf);
        let record = record.clone();
        // The work owns its single-flight completion. Dropping a list request
        // must not leave the slot pending after its blocking refresh finishes.
        tokio::spawn(async move {
            let task_cell = tracker_cell.clone();
            let cached_source = source.clone();
            let cached_state = state_dir.clone();
            let cached_record = record.clone();
            let cached_opener = tracker_opener.clone();
            let (cached, message) = tokio::task::spawn_blocking(move || {
                let Ok(mut tracker) = task_cell.lock() else {
                    return (ProviderLastPromptRefresh::Invalidated, None);
                };
                rebind_late_launch(
                    &mut tracker,
                    cached_state.as_deref(),
                    &cached_record,
                    &cached_source,
                    &cached_opener,
                );
                let Some(tracker) = tracker.as_mut() else {
                    return (ProviderLastPromptRefresh::Pending, None);
                };
                let refresh = tracker.cached_refresh();
                let message = if matches!(refresh, ProviderLastPromptRefresh::Current(_)) {
                    tracker.last_message()
                } else {
                    None
                };
                (refresh, message)
            })
            .await
            .unwrap_or((ProviderLastPromptRefresh::Invalidated, None));
            match cached {
                ProviderLastPromptRefresh::Current(prompt) => {
                    let mut state = slot.lock().await;
                    if Arc::ptr_eq(&state.last_prompt_tracker, &tracker_cell) {
                        state.last_prompt_in_flight = false;
                        state.last_prompt_invalidated = false;
                        Some(LastPromptProjection {
                            state: LastPromptState::Current,
                            continuity: Some(state.last_prompt_continuity.clone()),
                            prompt,
                            message,
                        })
                    } else {
                        Some(LastPromptProjection {
                            state: LastPromptState::Unavailable,
                            continuity: Some(state.last_prompt_continuity.clone()),
                            prompt: None,
                            message: None,
                        })
                    }
                }
                ProviderLastPromptRefresh::Pending => {
                    Self::spawn_last_prompt_refresh(
                        slot,
                        tracker_cell,
                        source,
                        tracker_opener,
                        recovery_permits,
                        state_dir,
                        record,
                    );
                    Some(LastPromptProjection {
                        state: if unavailable_until_current {
                            LastPromptState::Unavailable
                        } else {
                            LastPromptState::Pending
                        },
                        continuity: Some(continuity),
                        prompt: None,
                        message: None,
                    })
                }
                ProviderLastPromptRefresh::Invalidated => {
                    let mut state = slot.lock().await;
                    if Arc::ptr_eq(&state.last_prompt_tracker, &tracker_cell) {
                        state.source = None;
                        state.last_prompt_tracker = Arc::new(StdMutex::new(None));
                        state.last_prompt_in_flight = false;
                        state.last_prompt_invalidated = true;
                        state.last_prompt_continuity = uuid::Uuid::new_v4().simple().to_string();
                        state.next_scan_at = None;
                        state.backoff = PROVIDER_PROMPT_PENDING_POLL_INTERVAL;
                    }
                    Some(LastPromptProjection {
                        state: LastPromptState::Unavailable,
                        continuity: Some(state.last_prompt_continuity.clone()),
                        prompt: None,
                        message: None,
                    })
                }
            }
        })
        .await
        .ok()
        .flatten()
    }

    #[cfg(test)]
    pub(in crate::serve) async fn last_prompt(
        &self,
        record: &crate::SessionRecord,
    ) -> Option<LastPrompt> {
        self.last_prompt_projection(record, None)
            .await
            .and_then(|projection| projection.prompt)
    }

    fn spawn_last_prompt_refresh(
        slot: Arc<tokio::sync::Mutex<ProviderPromptDiscoverySlot>>,
        tracker_cell: Arc<StdMutex<Option<ProviderLastPromptTracker>>>,
        source: ProviderPromptSource,
        tracker_opener: Arc<ProviderLastPromptTrackerOpener>,
        recovery_permits: Arc<tokio::sync::Semaphore>,
        state_dir: Option<PathBuf>,
        record: crate::SessionRecord,
    ) {
        tokio::spawn(async move {
            loop {
                let Ok(_permit) = recovery_permits.clone().acquire_owned().await else {
                    let mut state = slot.lock().await;
                    if Arc::ptr_eq(&state.last_prompt_tracker, &tracker_cell) {
                        state.last_prompt_in_flight = false;
                        state.last_prompt_invalidated = true;
                        state.last_prompt_continuity = uuid::Uuid::new_v4().simple().to_string();
                    }
                    return;
                };
                let task_cell = tracker_cell.clone();
                let task_source = source.clone();
                let task_opener = tracker_opener.clone();
                let task_state = state_dir.clone();
                let task_record = record.clone();
                let refresh = tokio::task::spawn_blocking(move || {
                    let Ok(mut tracker) = task_cell.lock() else {
                        return ProviderLastPromptRefresh::Invalidated;
                    };
                    if tracker.is_none() {
                        let binding = task_state.as_deref().and_then(|state| {
                            let provider = task_record.provider_resume.as_ref()?;
                            crate::notice::launch::binding_for_source(
                                state,
                                &provider.provider,
                                &provider.session_id,
                                crate::session_agent_profile(&task_record),
                                task_source.path(),
                                std::time::Instant::now() + Duration::from_secs(2),
                            )
                        });
                        *tracker = task_opener(task_source.clone(), binding);
                    }
                    rebind_late_launch(
                        &mut tracker,
                        task_state.as_deref(),
                        &task_record,
                        &task_source,
                        &task_opener,
                    );
                    let Some(tracker) = tracker.as_mut() else {
                        return ProviderLastPromptRefresh::Invalidated;
                    };
                    tracker.refresh()
                })
                .await
                .unwrap_or(ProviderLastPromptRefresh::Invalidated);
                drop(_permit);

                let mut state = slot.lock().await;
                if !Arc::ptr_eq(&state.last_prompt_tracker, &tracker_cell) {
                    return;
                }
                match refresh {
                    ProviderLastPromptRefresh::Current(_) => {
                        state.last_prompt_in_flight = false;
                        return;
                    }
                    ProviderLastPromptRefresh::Pending => {
                        drop(state);
                        tokio::task::yield_now().await;
                    }
                    ProviderLastPromptRefresh::Invalidated => {
                        state.source = None;
                        state.last_prompt_tracker = Arc::new(StdMutex::new(None));
                        state.last_prompt_in_flight = false;
                        state.last_prompt_invalidated = true;
                        state.last_prompt_continuity = uuid::Uuid::new_v4().simple().to_string();
                        state.next_scan_at = None;
                        state.backoff = PROVIDER_PROMPT_PENDING_POLL_INTERVAL;
                        return;
                    }
                }
            }
        });
    }
}

fn rebind_late_launch(
    tracker: &mut Option<ProviderLastPromptTracker>,
    state: Option<&std::path::Path>,
    record: &crate::SessionRecord,
    source: &ProviderPromptSource,
    opener: &Arc<ProviderLastPromptTrackerOpener>,
) {
    if !tracker
        .as_ref()
        .is_some_and(ProviderLastPromptTracker::needs_launch_binding)
    {
        return;
    }
    let Some(state) = state else { return };
    if !state
        .join("sessions")
        .join(&record.id)
        .join("launch-notice.json")
        .is_file()
    {
        return;
    }
    let Some(provider) = record.provider_resume.as_ref() else {
        return;
    };
    if let Some(binding) = crate::notice::launch::binding_for_source(
        state,
        &provider.provider,
        &provider.session_id,
        crate::session_agent_profile(record),
        source.path(),
        std::time::Instant::now() + Duration::from_secs(2),
    ) {
        *tracker = opener(source.clone(), Some(binding));
    }
}
