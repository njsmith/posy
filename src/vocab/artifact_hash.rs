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
        let gold_data = b"123456890";
        let good_hash = ArtifactHash::from_hex(
            "sha256",
            "c775e7b757ede630cd0aa1113bd102661ab38829ca52a6422ab782862f268646",
        ).unwrap();
        let bad_hash = ArtifactHash::from_hex(
            "sha256",
            "775e7b757ede630cd0aa1113bd102661ab38829ca52a6422ab782862f268646c",
        ).unwrap();
        let mut buf = [0u8; 20];

        let mut good_checker = HashChecker::new(&good_hash, buf.as_mut()).unwrap();
        assert!(good_checker.write_all(gold_data).is_ok());
        assert!(good_checker.flush().is_ok());
        let mut unwrapped = good_checker.finish().unwrap();
        assert_eq!(&buf[0..10], gold_data);
        assert_eq!(&buf[10..20], &[0u8; 10]);

        let mut buf = [0u8; 20];
        let mut bad_checker = HashChecker::new(&bad_hash, buf.as_mut()).unwrap();
        assert!(bad_checker.write_all(gold_data).is_ok());
        assert!(bad_checker.flush().is_ok());
        assert!(bad_checker.finish().is_err());
    }
}
