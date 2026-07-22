use super::{RawEvidence, SourceError, SubQuestion};

pub struct SportsProvider;

impl SportsProvider {
    pub fn fetch(&self, _sub_q: &SubQuestion) -> Result<RawEvidence, SourceError> {
        Err(SourceError::Empty)
    }
}
