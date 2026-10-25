//! Bounded incremental prompt and message projection for session lists.
use super::*;

impl ProviderPromptDiscoveryRegistry {
    pub(in crate::serve) async fn last_prompt_projection(
        &self,
        record: &crate::SessionRecord,
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
        let task_cell = tracker_cell.clone();
        let (cached, message) = tokio::task::spawn_blocking(move || {
            let Ok(mut tracker) = task_cell.lock() else {
                return (ProviderLastPromptRefresh::Invalidated, None);
            };
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
                self.spawn_last_prompt_refresh(slot, tracker_cell, source);
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
                    state.last_prompt_in_flight = false;
                }
                drop(state);
                self.invalidate_source(record).await;
                Some(LastPromptProjection {
                    state: LastPromptState::Unavailable,
                    continuity: self.continuity_for_key(&key).await,
                    prompt: None,
                    message: None,
                })
            }
        }
    }

    #[cfg(test)]
    pub(in crate::serve) async fn last_prompt(
        &self,
        record: &crate::SessionRecord,
    ) -> Option<LastPrompt> {
        self.last_prompt_projection(record)
            .await
            .and_then(|projection| projection.prompt)
    }

    fn spawn_last_prompt_refresh(
        &self,
        slot: Arc<tokio::sync::Mutex<ProviderPromptDiscoverySlot>>,
        tracker_cell: Arc<StdMutex<Option<ProviderLastPromptTracker>>>,
        source: ProviderPromptSource,
    ) {
        let tracker_opener = self.tracker_opener.clone();
        let recovery_permits = self.recovery_permits.clone();
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
                let refresh = tokio::task::spawn_blocking(move || {
                    let Ok(mut tracker) = task_cell.lock() else {
                        return ProviderLastPromptRefresh::Invalidated;
                    };
                    if tracker.is_none() {
                        *tracker = task_opener(task_source);
                    }
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
