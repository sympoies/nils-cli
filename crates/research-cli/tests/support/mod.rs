use nils_research_cli::{
    fetch::PublicReader,
    transport::{ProviderFuture, Transport},
    *,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

pub fn fixture(provider: Provider, scenario: &str) -> Value {
    let prefix = match provider {
        Provider::Openai => "openai",
        Provider::Anthropic => "anthropic",
    };
    serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/tests/fixtures/{prefix}-{scenario}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}
pub struct FixtureTransport {
    provider: Provider,
    scenario: String,
    pub calls: Mutex<Vec<Value>>,
}
impl Transport for FixtureTransport {
    fn post<'a>(&'a self, provider: Provider, body: Value, _: &'a Limits) -> ProviderFuture<'a> {
        Box::pin(async move {
            assert_eq!(provider, self.provider);
            self.calls.lock().unwrap().push(body);
            let wire = fixture(provider, &self.scenario);
            match self.scenario.as_str() {
                "429" => Err(ResearchError::status(
                    wire["http_status"].as_u64().unwrap() as u16,
                    None,
                )),
                "timeout" => {
                    tokio::time::sleep(Duration::from_millis(wire["delay_ms"].as_u64().unwrap()))
                        .await;
                    Ok(json!({}))
                }
                _ => Ok(wire),
            }
        })
    }
}
pub fn engine(provider: Provider, scenario: &str) -> (Engine, Arc<FixtureTransport>) {
    let transport = Arc::new(FixtureTransport {
        provider,
        scenario: scenario.into(),
        calls: Mutex::new(vec![]),
    });
    (
        Engine::new(transport.clone(), Arc::new(PublicReader::default())),
        transport,
    )
}
