use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash, SerializeDisplay)]
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

    pub fn checker<'a, T: Write>(&'a self, inner: T) -> Result<HashChecker<'a, T>> {
        let algorithm = match self.mode.as_str() {
            "sha256" => &ring::digest::SHA256,
            _ => bail!("unknown hash algorithm {self.mode}"),
        };
        let state = ring::digest::Context::new(algorithm);
        Ok(HashChecker {
            inner,
            state,
            expected: self,
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

pub struct HashChecker<'a, T: Write> {
    inner: T,
    state: ring::digest::Context,
    expected: &'a ArtifactHash,
}

impl<'a, T: Write> HashChecker<'a, T> {
    pub fn finish(self) -> Result<T> {
        let digest = self.state.finish();
        if &self.expected.raw_data != digest.as_ref() {
            bail!("hash mismatch: {:?} != {:?}", self.expected, digest);
        }
        Ok(self.inner)
    }
}

impl<'a, T: Write> Write for HashChecker<'a, T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(&buf)?;
        println!("update {:?}", &buf[..written]);
        self.state.update(&buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
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
        )
        .unwrap();
        assert_eq!(
            value.to_string(),
            "sha256=c27c231e66336183c484fbfe080fa6cc954149366c15dc21db8b7290081ec7b8"
        );
    }

    #[test]
    fn test_hash_checker() {
        let gold_data = b"a drop of golden sun";
        let good_hash = ArtifactHash::from_hex(
            "sha256",
            "9c7ed1509d1809656c86aa1201fde2650ec056ab79f6546ba8205f6e42cff949",
        ).unwrap();
        let bad_hash = ArtifactHash::from_hex(
            "sha256",
            "007ed1509d1809656c86aa1201fde2650ec056ab79f6546ba8205f6e42cff949",
        ).unwrap();

        let buf = Vec::<u8>::new();
        let mut good_checker = good_hash.checker(buf).unwrap();
        assert!(good_checker.write_all(gold_data).is_ok());
        assert!(good_checker.flush().is_ok());
        let unwrapped = good_checker.finish().unwrap();
        assert_eq!(unwrapped.as_slice(), gold_data);

        let buf = Vec::<u8>::new();
        let mut bad_checker = bad_hash.checker(buf).unwrap();
        assert!(bad_checker.write_all(gold_data).is_ok());
        assert!(bad_checker.flush().is_ok());
        assert!(bad_checker.finish().is_err());
    }
}
