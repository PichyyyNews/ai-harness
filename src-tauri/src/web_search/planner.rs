use super::{ProviderKind, QueryPlan, RawEvidence, SourceHint, SubQuestion};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct JudgeReply {
    decision: String,
}

#[derive(Debug, Deserialize)]
struct SourceArguments {
    location: Option<String>,
    currency_from: Option<String>,
    currency_to: Option<String>,
    asset_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NoEvidenceAnswer {
    answer: String,
}

/// A separate, compact AI worker selects retrieval capabilities from metadata.
/// It never relies on per-language or per-topic keyword routing in the app.
pub fn choose_providers(endpoint: &str, plan: &QueryPlan) -> Option<Vec<Vec<ProviderKind>>> {
    let choices = plan
        .sub_questions
        .iter()
        .map(|question| choose_for_question(endpoint, &question.text))
        .collect::<Option<Vec<_>>>()?;
    Some(choices)
}

fn choose_for_question(endpoint: &str, question: &str) -> Option<Vec<ProviderKind>> {
    let prompt = format!(
        "Pick up to 3 source IDs that best answer the query. Reply only comma-separated IDs. Catalog: wiki=encyclopedia; wd=entity facts; arxiv=research; sem=scholarly papers; coin=crypto; weather=forecast; map=places; git=software; stack=developer Q&A; nvd=vulnerabilities; countries=country facts; fx=exchange rates; news=current events; web=general web. Query: {}",
        question.chars().take(360).collect::<String>()
    );
    let response = call_text(endpoint, &prompt, 32)?;
    let providers = parse_provider_ids(&response);
    (!providers.is_empty()).then_some(providers)
}

fn parse_provider_ids(value: &str) -> Vec<ProviderKind> {
    let catalog = [
        ("wiki", ProviderKind::Wikipedia),
        ("wd", ProviderKind::Wikidata),
        ("arxiv", ProviderKind::Arxiv),
        ("sem", ProviderKind::SemanticScholar),
        ("coin", ProviderKind::CoinGecko),
        ("weather", ProviderKind::OpenMeteo),
        ("map", ProviderKind::OpenStreetMap),
        ("git", ProviderKind::GitHub),
        ("stack", ProviderKind::StackExchange),
        ("nvd", ProviderKind::Nvd),
        ("countries", ProviderKind::RestCountries),
        ("fx", ProviderKind::ExchangeRate),
        ("news", ProviderKind::GoogleNews),
        ("web", ProviderKind::GeneralWeb),
    ];
    value
        .to_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter_map(|token| {
            catalog
                .iter()
                .find_map(|(id, provider)| (*id == token).then_some(*provider))
        })
        .fold(Vec::new(), |mut providers, provider| {
            if providers.len() < 3 && !providers.contains(&provider) {
                providers.push(provider);
            }
            providers
        })
}

/// Tier 1 extracts provider parameters by meaning. The app never attempts to
/// maintain city/currency/asset keyword lists for every language or dialect.
pub fn extract_source_hint(
    endpoint: &str,
    question: &str,
    providers: &[ProviderKind],
) -> Option<SourceHint> {
    let target = providers.iter().find(|provider| {
        matches!(
            provider,
            ProviderKind::OpenMeteo | ProviderKind::ExchangeRate | ProviderKind::CoinGecko
        )
    })?;
    let requested = match target {
        ProviderKind::OpenMeteo => "location: the place name only",
        ProviderKind::ExchangeRate => {
            "currency_from and currency_to: ISO 4217 three-letter codes only"
        }
        ProviderKind::CoinGecko => {
            "asset_id: the lowercase CoinGecko asset id, for example bitcoin or ethereum"
        }
        _ => return None,
    };
    let prompt = format!(
        "Extract parameters for a live-data provider from the user's request in whatever language or dialect it uses. Return JSON only with this exact shape: {{\"location\":string|null,\"currency_from\":string|null,\"currency_to\":string|null,\"asset_id\":string|null}}. Fill only the requested fields ({requested}); use null when the request does not specify them. User request: {}",
        question.chars().take(800).collect::<String>()
    );
    let raw = call_json(endpoint, &prompt, 96)?;
    let arguments: SourceArguments = serde_json::from_str(&raw).ok()?;
    match target {
        ProviderKind::OpenMeteo => arguments
            .location
            .filter(|value| !value.trim().is_empty())
            .map(|location_text| SourceHint::Weather { location_text }),
        ProviderKind::ExchangeRate => {
            let from = normalized_currency(arguments.currency_from?)?;
            let to = normalized_currency(arguments.currency_to?)?;
            Some(SourceHint::Currency { from, to })
        }
        ProviderKind::CoinGecko => arguments
            .asset_id
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
            .map(|ticker| SourceHint::StockOrCrypto { ticker }),
        _ => None,
    }
}

fn normalized_currency(value: String) -> Option<String> {
    let code = value.trim().to_ascii_uppercase();
    (code.len() == 3
        && code
            .chars()
            .all(|character| character.is_ascii_alphabetic()))
    .then_some(code)
}

/// The judge spends an inference call only where deterministic evidence scoring
/// is borderline. This keeps small local models responsive while still letting
/// AI decide whether one bounded refinement is worth doing.
pub fn should_refine(endpoint: &str, sub_q: &SubQuestion, evidence: &RawEvidence) -> Option<bool> {
    let evidence_text = evidence
        .chunks
        .iter()
        .take(3)
        .map(|chunk| chunk.text.chars().take(360).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!("You judge evidence quality. Return JSON only: {{\"decision\":\"sufficient\"}} or {{\"decision\":\"refine\"}}. Question: {}\nEvidence:\n{}", sub_q.text, evidence_text);
    let value = call_json(endpoint, &prompt, 64)?;
    let parsed: JudgeReply = serde_json::from_str(&value).ok()?;
    match parsed.decision.as_str() {
        "sufficient" => Some(false),
        "refine" => Some(true),
        _ => None,
    }
}

/// Produces only a localized search-outcome notice. The caller uses this when
/// all live providers returned zero evidence, preventing the chat model from
/// filling the gap with remembered or invented current events.
pub fn no_evidence_answer(endpoint: &str, question: &str) -> Option<String> {
    let prompt = format!(
        "Return JSON only: {{\"answer\":\"...\"}}. Write one concise sentence in the same language as the request saying that the live search completed but returned no usable current evidence and asking for a more specific query. Do not provide facts, examples, trends, or citations. Request: {}",
        question.chars().take(800).collect::<String>()
    );
    let raw = call_json(endpoint, &prompt, 128)?;
    let answer = serde_json::from_str::<NoEvidenceAnswer>(&raw)
        .ok()?
        .answer
        .trim()
        .to_string();
    (!answer.is_empty()).then_some(answer)
}

fn call_json(endpoint: &str, prompt: &str, max_tokens: u32) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .ok()?;
    let request = |messages: serde_json::Value| -> Option<String> {
        let response: serde_json::Value = client
            .post(format!("{endpoint}/v1/chat/completions"))
            .json(&serde_json::json!({
                "messages": messages,
                "max_tokens": max_tokens,
                "temperature": 0.0,
                "stream": false,
                "response_format": {"type":"json_object"},
                "chat_template_kwargs": {"enable_thinking": false}
            }))
            .send()
            .ok()?
            .error_for_status()
            .ok()?
            .json()
            .ok()?;
        response
            .pointer("/choices/0/message/content")?
            .as_str()
            .map(str::trim)
            .filter(|content| !content.is_empty())
            .map(ToOwned::to_owned)
    };

    // Some small local models return an empty completion when a system message
    // is present. Retry in a user-only shape before falling back to rules.
    let content = request(serde_json::json!([
        {"role":"system","content":"Follow the requested JSON schema exactly. No Markdown."},
        {"role":"user","content":prompt}
    ]))
    .or_else(|| {
        request(serde_json::json!([
            {"role":"user","content":format!("Return JSON only. {prompt}")}
        ]))
    })?;
    Some(
        content
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string(),
    )
}

fn call_text(endpoint: &str, prompt: &str, max_tokens: u32) -> Option<String> {
    let response: serde_json::Value = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?
        .post(format!("{endpoint}/v1/chat/completions"))
        .json(&serde_json::json!({
            "messages": [{"role":"user","content":prompt}],
            "max_tokens": max_tokens,
            "temperature": 0.0,
            "stream": false,
            "chat_template_kwargs": {"enable_thinking": false}
        }))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .ok()?;
    response
        .pointer("/choices/0/message/content")?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_capability_ids_without_query_keyword_rules() {
        let providers = parse_provider_ids("news, web, fx");
        assert_eq!(
            providers,
            vec![
                ProviderKind::GoogleNews,
                ProviderKind::GeneralWeb,
                ProviderKind::ExchangeRate
            ]
        );
        assert!(parse_provider_ids("sometimes a model explains itself").is_empty());
    }

    #[test]
    fn validates_structured_currency_codes() {
        assert_eq!(
            normalized_currency("thb".to_string()).as_deref(),
            Some("THB")
        );
        assert!(normalized_currency("baht".to_string()).is_none());
    }
}
