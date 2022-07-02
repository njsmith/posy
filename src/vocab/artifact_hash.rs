use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtifactHash {
    pub mode: String,
    pub raw_data: Vec<u8>,
}

impl ArtifactHash {
    pub fn from_hex(mode: &str, hex: &str) -> Result<ArtifactHash> {
        Ok(ArtifactHash {
            mode: mode.into(),
            raw_data: data_encoding::HEXLOWER_PERMISSIVE.decode(hex.as_bytes())?,
        })
    }
}

impl Display for ArtifactHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}={}",
            self.mode,
            data_encoding::HEXLOWER.encode(&self.raw_data),
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_artifact_hash() {
        let value = ArtifactHash::from_hex(
            "sha256",
            "c27c231e66336183c484fbfe080fa6cc954149366c15dc21db8b7290081ec7b8",
        ).unwrap();
        assert_eq!(value.to_string(),
            "sha256=c27c231e66336183c484fbfe080fa6cc954149366c15dc21db8b7290081ec7b8");
    }
}
