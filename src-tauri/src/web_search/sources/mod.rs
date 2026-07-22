use super::{RawEvidence, SubQuestion};
use crate::web_search::ProviderKind;
use std::fmt;

#[allow(dead_code)]
pub mod currency;
pub mod news;
pub mod open_data;
#[allow(dead_code)]
pub mod registry;
#[allow(dead_code)]
pub mod sports;
pub mod stocks;
pub mod weather;
pub mod wikipedia;

use currency::CurrencyProvider;
use news::NewsProvider;
use registry::RegistryProvider;
use sports::SportsProvider;
use stocks::StocksProvider;
use weather::WeatherProvider;
use wikipedia::WikipediaProvider;

#[derive(Debug)]
pub enum SourceError {
    Timeout,
    FetchFailed(String),
    Empty,
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Timeout => write!(f, "Source fetch timed out"),
            SourceError::FetchFailed(msg) => write!(f, "Source fetch failed: {}", msg),
            SourceError::Empty => write!(f, "Source returned empty evidence"),
        }
    }
}

impl std::error::Error for SourceError {}

#[allow(dead_code)]
pub enum DedicatedProvider {
    Wikipedia(WikipediaProvider),
    Weather(WeatherProvider),
    Currency(CurrencyProvider),
    Stocks(StocksProvider),
    Sports(SportsProvider),
    News(NewsProvider),
    Registry(RegistryProvider),
}

#[allow(dead_code)]
impl DedicatedProvider {
    pub fn fetch(&self, sub_q: &SubQuestion) -> Result<RawEvidence, SourceError> {
        match self {
            DedicatedProvider::Wikipedia(p) => p.fetch(sub_q),
            DedicatedProvider::Weather(p) => p.fetch(sub_q),
            DedicatedProvider::Currency(p) => p.fetch(sub_q),
            DedicatedProvider::Stocks(p) => p.fetch(sub_q),
            DedicatedProvider::Sports(p) => p.fetch(sub_q),
            DedicatedProvider::News(p) => p.fetch(sub_q),
            DedicatedProvider::Registry(p) => p.fetch(sub_q),
        }
    }
}

pub fn fetch_kind(kind: ProviderKind, sub_q: &SubQuestion) -> Result<RawEvidence, SourceError> {
    match kind {
        ProviderKind::Wikipedia => WikipediaProvider.fetch(sub_q),
        ProviderKind::OpenMeteo => WeatherProvider.fetch(sub_q),
        ProviderKind::CoinGecko => StocksProvider.fetch(sub_q),
        ProviderKind::ExchangeRate => CurrencyProvider.fetch(sub_q),
        ProviderKind::GoogleNews => NewsProvider.fetch(sub_q),
        ProviderKind::GeneralWeb => Err(SourceError::Empty),
        ProviderKind::Wikidata
        | ProviderKind::Arxiv
        | ProviderKind::SemanticScholar
        | ProviderKind::OpenStreetMap
        | ProviderKind::GitHub
        | ProviderKind::StackExchange
        | ProviderKind::Nvd
        | ProviderKind::RestCountries => open_data::OpenDataProvider.fetch(kind, sub_q),
    }
}
