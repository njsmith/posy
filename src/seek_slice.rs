use std::io;
use std::io::{Read, Seek, SeekFrom};

pub struct SeekSlice<T: Seek> {
    inner: T,
    start: u64,
    end: u64,
    current: u64,
}

impl<T: Seek> SeekSlice<T> {
    pub fn new(inner: T, start: u64, end: u64) -> std::io::Result<SeekSlice<T>> {
        assert!(end >= start);
        Ok(SeekSlice {
            inner,
            start,
            end,
            current: inner.seek(SeekFrom::Start(start))?,
        })
    }
}

impl<T: Seek> Seek for SeekSlice<T> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let mut goal_idx = match pos {
            SeekFrom::Start(amount) => self.start + amount,
            SeekFrom::End(amount) => self.end + amount,
            SeekFrom::Current(amount) => self.current + amount,
        };
        if goal_idx < self.start || goal_idx >= self.end {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a negative or overflowing position",
                ))
        }
        self.current = self.inner.seek(SeekFrom::Start(goal_idx))?;
        Ok(self.current - self.end)
    }
}

impl<T: Read + Seek> Read for SeekSlice<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let max_read = self.end - self.current;
        let amount = self.inner.read(&buf[..max_read])?;
        self.current += amount;
        Ok(amount)
    }
}

// could impl Write as well, but so far I haven't needed it

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_seek_slice() {
        let buf: [u8] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let mut cursor = Cursor::new(&buf);
        let mut slice = SeekSlice::new(&mut cursor, 2, 8).unwrap();
        // starts at offset zero
        assert_eq!(slice.seek(SeekFrom::Current(0)).unwrap(), 0);
        // reading advances position as expected
        assert_eq!(slice.bytes().next(), Some(2u8));
        assert_eq!(slice.bytes().next(), Some(3u8));
        assert_eq!(slice.seek(SeekFrom::Current(0)).unwrap(), 2);
        assert_eq!(slice.bytes().next(), Some(4u8));

        // out of range seeks caught and have no effect
        assert!(slice.seek(SeekFrom::Current(-10)).is_err());
        assert!(slice.seek(SeekFrom::Current(10)).is_err());
        assert_eq!(slice.bytes().next(), Some(5u8));

        assert_eq!(slice.seek(SeekFrom::Start(1)).unwrap(), 1);
        assert_eq!(slice.bytes().next(), Some(1));

        assert_eq!(slice.seek(SeekFrom::End(-1)).unwrap(), 5);
        assert_eq!(slice.bytes().next(), Some(7));
        assert_eq!(slice.bytes().next(), None);
    }
}
