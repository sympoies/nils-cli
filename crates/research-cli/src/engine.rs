use crate::{
    Operation, Provider, Request, ResearchError, ResearchResult, Source,
    fetch::{Page, PublicReader, UrlReader, parse_public_url},
    providers,
    transport::{ApiTransport, Transport},
};
use nils_common::cli_contract::Envelope;
use std::{sync::Arc, time::Duration};
use tokio::time::{Instant, timeout_at};

#[derive(Clone)]
pub struct Engine {
    transport: Arc<dyn Transport>,
    reader: Arc<dyn UrlReader>,
}
impl Default for Engine {
    fn default() -> Self {
        Self::new(Arc::new(ApiTransport), Arc::new(PublicReader::default()))
    }
}
impl Engine {
    pub fn new(transport: Arc<dyn Transport>, reader: Arc<dyn UrlReader>) -> Self {
        Self { transport, reader }
    }
    pub async fn execute(&self, request: Request) -> Envelope<ResearchResult> {
        if let Err(error) = request.validate() {
            return error.envelope(request.operation, None);
        }
        let mut result = ResearchResult::empty(&request);
        let outcome = self.run(&request, &mut result).await;
        bound_output(&mut result, request.limits.max_chars);
        match outcome {
            Ok(()) => Envelope::success(request.operation.schema(), result.clone())
                .with_warnings(result.warnings),
            Err(error) => error.envelope(request.operation, Some(result)),
        }
    }
    async fn run(
        &self,
        request: &Request,
        result: &mut ResearchResult,
    ) -> Result<(), ResearchError> {
        for url in &request.urls {
            parse_public_url(url)?;
        }
        let deadline = Instant::now() + Duration::from_secs(request.limits.timeout_seconds);
        let mut pages: Vec<Page> = Vec::new();
        for url in &request.urls {
            let page = timeout_at(deadline, self.reader.read(url, &request.limits))
                .await
                .map_err(|_| {
                    ResearchError::new("timeout", "Research deadline exceeded while reading a URL")
                })??;
            result.truncated |= page.truncated;
            if page.truncated {
                result.warnings.push(
                    "Public URL byte or character limit reached; only read content is retained"
                        .into(),
                );
            }
            result.sources.push(Source {
                id: format!("s{}", result.sources.len() + 1),
                url: page.url.clone(),
                final_url: Some(page.final_url.clone()),
                title: None,
                published_at: None,
                excerpt: Some(page.content.clone()),
            });
            pages.push(page);
        }
        if request.operation == Operation::ReadUrl {
            result.content = pages.pop().map(|p| p.content);
            return Ok(());
        }
        let prompt = providers::prompt(request, &pages);
        let body = match request.provider {
            Provider::Openai => providers::openai::request(request, &prompt),
            Provider::Anthropic => providers::anthropic::request(request, &prompt),
        };
        let wire = timeout_at(
            deadline,
            self.transport.post(request.provider, body, &request.limits),
        )
        .await
        .map_err(|_| {
            ResearchError::new(
                "timeout",
                "Research provider deadline exceeded; billing may be uncertain",
            )
        })??;
        match request.provider {
            Provider::Openai => providers::openai::parse(request, &wire, result),
            Provider::Anthropic => providers::anthropic::parse(request, &wire, result),
        }
    }
}

// One character budget across prose fields. Keep identity URLs and IDs intact.
fn bound_output(result: &mut ResearchResult, max_chars: usize) {
    let mut remaining = max_chars;
    let mut truncated = result.truncated;
    let mut trim = |s: &mut String| {
        let length = s.chars().count();
        if length > remaining {
            *s = s.chars().take(remaining).collect();
            truncated = true;
            remaining = 0;
        } else {
            remaining -= length;
        }
    };
    if let Some(content) = &mut result.content {
        trim(content);
    }
    for source in &mut result.sources {
        // Plain reads expose content once, avoiding duplicate output against the budget.
        if result.content.is_some() {
            source.excerpt = None;
        }
        if let Some(title) = &mut source.title {
            trim(title);
        }
    }
    for fact in &mut result.facts {
        trim(&mut fact.claim);
    }
    result.facts.retain(|f| !f.claim.is_empty());
    if let Some(summary) = &mut result.summary {
        trim(summary);
    }
    for item in result
        .assessment
        .key_points
        .iter_mut()
        .chain(result.assessment.suggested_directions.iter_mut())
        .chain(result.unknowns.iter_mut())
        .chain(result.conflicts.iter_mut())
    {
        trim(item);
    }
    if let Some(judgment) = &mut result.assessment.preliminary_judgment {
        trim(judgment);
    }
    for source in &mut result.sources {
        if let Some(excerpt) = &mut source.excerpt {
            trim(excerpt);
        }
    }
    if truncated && !result.truncated {
        result
            .warnings
            .push("Output character budget reached".into());
    }
    result.truncated = truncated;
}
