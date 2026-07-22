use super::SourceError;
use crate::web_search::{EvidenceChunk, RawEvidence, SourceHint, SourceKind, SubQuestion};
use serde::Deserialize;
use std::time::Duration;

pub struct WeatherProvider;

impl WeatherProvider {
    /// Open-Meteo is keyless and split into geocoding plus forecast endpoints.
    /// We deliberately use only a location expressed in the user's query.
    pub fn fetch(&self, sub_q: &SubQuestion) -> Result<RawEvidence, SourceError> {
        let location = match &sub_q.source_hint {
            SourceHint::Weather { location_text } => location_text,
            _ => &sub_q.text,
        };
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .user_agent("AI Harness retrieval/1.0")
            .build()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
        #[derive(Deserialize)]
        struct Geo {
            results: Option<Vec<Place>>,
        }
        #[derive(Deserialize)]
        struct Place {
            name: String,
            latitude: f64,
            longitude: f64,
            country: Option<String>,
        }
        let geo_params = vec![("name", location.to_string()), ("count", "1".to_string())];
        let geo: Geo = client
            .get("https://geocoding-api.open-meteo.com/v1/search")
            .query(&geo_params)
            .send()
            .map_err(map_error)?
            .error_for_status()
            .map_err(map_error)?
            .json()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
        let place = geo
            .results
            .and_then(|mut values| values.drain(..).next())
            .ok_or(SourceError::Empty)?;
        #[derive(Deserialize)]
        struct Forecast {
            current: Option<Current>,
        }
        #[derive(Deserialize)]
        struct Current {
            temperature_2m: Option<f64>,
            relative_humidity_2m: Option<f64>,
            wind_speed_10m: Option<f64>,
            weather_code: Option<i64>,
            time: Option<String>,
        }
        let forecast_params = vec![
            ("latitude", place.latitude.to_string()),
            ("longitude", place.longitude.to_string()),
            (
                "current",
                "temperature_2m,relative_humidity_2m,wind_speed_10m,weather_code".to_string(),
            ),
        ];
        let forecast: Forecast = client
            .get("https://api.open-meteo.com/v1/forecast")
            .query(&forecast_params)
            .send()
            .map_err(map_error)?
            .error_for_status()
            .map_err(map_error)?
            .json()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
        let current = forecast.current.ok_or(SourceError::Empty)?;
        let text = format!("Open-Meteo current weather for {}, {}: {}°C, humidity {}%, wind {} km/h, WMO code {}, observed {}.", place.name, place.country.unwrap_or_default(), current.temperature_2m.unwrap_or_default(), current.relative_humidity_2m.unwrap_or_default(), current.wind_speed_10m.unwrap_or_default(), current.weather_code.unwrap_or_default(), current.time.unwrap_or_default());
        Ok(RawEvidence {
            chunks: vec![EvidenceChunk {
                text,
                source_url: format!(
                    "https://open-meteo.com/en/docs#latitude={}&longitude={}",
                    place.latitude, place.longitude
                ),
                source_title: format!("Open-Meteo: {}", place.name),
                host: "open-meteo.com".to_string(),
            }],
            source_kind: SourceKind::Dedicated("OpenMeteo".to_string()),
        })
    }
}

fn map_error(error: reqwest::Error) -> SourceError {
    if error.is_timeout() {
        SourceError::Timeout
    } else {
        SourceError::FetchFailed(error.to_string())
    }
}
