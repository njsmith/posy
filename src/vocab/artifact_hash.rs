use crate::prelude::*;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum HashMode {
    SHA256,
}

impl Display for HashMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashMode::SHA256 => "sha256",
        }
        .fmt(f)
    }
}

impl TryFrom<&str> for HashMode {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(match &value[..] {
            "sha256" => HashMode::SHA256,
            _ => bail!("unrecognized hash function {:?}", value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub struct ArtifactHash {
    pub mode: HashMode,
    pub raw_data: Vec<u8>,
}

impl ArtifactHash {
    pub fn from_hex(mode: HashMode, hex: &str) -> Result<ArtifactHash> {
        Ok(ArtifactHash {
            mode,
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

impl TryFrom<&str> for ArtifactHash {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.split_once('=') {
            None => bail!("expected '=' in hash specifier {:?}", value),
            Some((mode_str, hex_str)) => {
                ArtifactHash::from_hex(mode_str.try_into()?, hex_str)
            }
        }
    }
}

try_from_str_boilerplate!(ArtifactHash);

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_sha256_roundtrip() {
        let value =
            "sha256=c27c231e66336183c484fbfe080fa6cc954149366c15dc21db8b7290081ec7b8";
        let obj: ArtifactHash = value.try_into().unwrap();
        assert_eq!(obj.to_string(), value);
    }
}
