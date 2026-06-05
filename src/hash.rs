use std::io::SeekFrom;

use binrw::io::{Read, Seek, Write};

#[cfg(feature = "use_openssl")]
pub struct HashWriter<T> {
    sha1: Option<openssl::sha::Sha1>,
    sha256: Option<openssl::sha::Sha256>,
    data: T,
}

#[cfg(not(feature = "use_openssl"))]
pub struct HashWriter<T> {
    sha1: Option<sha1::Sha1>,
    sha256: Option<sha2::Sha256>,
    data: T,
}

impl<T> HashWriter<T>
where
    T: Write + Seek + Send,
{
    #[cfg(feature = "use_openssl")]
    pub fn new(data: T) -> Self {
        HashWriter {
            sha1: Some(openssl::sha::Sha1::new()),
            sha256: Some(openssl::sha::Sha256::new()),
            data,
        }
    }

    #[cfg(not(feature = "use_openssl"))]
    pub fn new(data: T) -> Self {
        use sha1::Digest;

        HashWriter {
            sha1: Some(sha1::Sha1::new()),
            sha256: Some(sha2::Sha256::new()),
            data,
        }
    }

    pub fn disable(&mut self) {
        self.sha1 = None;
        self.sha256 = None;
    }

    #[cfg(feature = "use_openssl")]
    pub fn hash(&mut self) -> ([u8; 20], [u8; 32]) {
        if let (Some(sha1), Some(sha2)) = (self.sha1.take(), self.sha256.take()) {
            return (sha1.finish(), sha2.finish());
        }
        ([0u8; 20], [0u8; 32])
    }

    #[cfg(not(feature = "use_openssl"))]
    pub fn hash(&mut self) -> ([u8; 20], [u8; 32]) {
        if let (Some(sha1), Some(sha2)) = (self.sha1.take(), self.sha256.take()) {
            use sha1::Digest;
            let sha1_vec = sha1.finalize().to_vec();
            let sha2_vec = sha2.finalize().to_vec();
            let mut s1 = [0u8; 20];
            let mut s2 = [0u8; 32];
            s1.copy_from_slice(&sha1_vec);
            s2.copy_from_slice(&sha2_vec);
            return (s1, s2);
        }
        ([0u8; 20], [0u8; 32])
    }

    pub fn into_inner(self) -> T {
        self.data
    }
}
impl<T> Seek for HashWriter<T>
where
    T: Write + Seek + Send,
{
    fn seek(&mut self, pos: SeekFrom) -> impl Future<Output = std::io::Result<u64>> + Send {
        self.data.seek(pos)
    }
}
impl<T> Write for HashWriter<T>
where
    T: Write + Seek + Send,
{
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = std::io::Result<usize>> + Send {
        async move {
            let size = self.data.write(buf).await?;
            if let (Some(sha1), Some(sha2)) = (&mut self.sha1, &mut self.sha256) {
                #[cfg(feature = "use_openssl")]
                {
                    sha1.update(&buf[..size]);
                    sha2.update(&buf[..size]);
                }
                #[cfg(not(feature = "use_openssl"))]
                {
                    use sha1::digest::DynDigest;
                    sha1.update(&buf[..size]);
                    sha2.update(&buf[..size]);
                }
            }
            Ok(size)
        }
    }

    fn flush(&mut self) -> impl Future<Output = std::io::Result<()>> + Send {
        async move {
            self.data.flush().await?;
            Ok(())
        }
    }
}
pub struct Crc32Reader<R: Read + Send> {
    inner: R,
    crc32: Option<crc32fast::Hasher>,
}
impl<T> Crc32Reader<T>
where
    T: Read + Send,
{
    pub fn new(reader: T) -> Self {
        Crc32Reader {
            inner: reader,
            crc32: None,
        }
    }
    pub fn init_crc32(&mut self) {
        self.crc32 = Some(crc32fast::Hasher::new())
    }
    pub fn crc32(self) -> u32 {
        if let Some(crc) = self.crc32 {
            crc.finalize()
        } else {
            0
        }
    }
}
impl<T> Read for Crc32Reader<T>
where
    T: Read + Send,
{
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = std::io::Result<usize>> + Send {
        async move {
            let size = self.inner.read(buf).await?;
            if let Some(crc32) = &mut self.crc32 {
                crc32.update(&buf[..size]);
            }
            Ok(size)
        }
    }

    fn flush(&mut self) -> impl Future<Output = std::io::Result<()>> + Send {
        self.inner.flush()
    }
}
pub trait Hasher: Send + binrw::io::write::Write {
    fn new() -> Self;
    fn update(&mut self, data: &[u8]) -> std::io::Result<()>;
    fn finalize(self) -> ([u8; 20], [u8; 32]);
}

#[cfg(feature = "use_openssl")]
pub struct HashWriterNull {
    sha1: openssl::sha::Sha1,
    sha256: openssl::sha::Sha256,
}

#[cfg(not(feature = "use_openssl"))]
pub struct HashWriterNull {
    sha1: sha1::Sha1,
    sha256: sha2::Sha256,
}

impl Hasher for HashWriterNull {
    fn new() -> Self {
        #[cfg(feature = "use_openssl")]
        {
            Self {
                sha1: openssl::sha::Sha1::new(),
                sha256: openssl::sha::Sha256::new(),
            }
        }
        #[cfg(not(feature = "use_openssl"))]
        {
            use sha1::Digest;

            Self {
                sha1: sha1::Sha1::new(),
                sha256: sha2::Sha256::new(),
            }
        }
    }

    fn update(&mut self, data: &[u8]) -> std::io::Result<()> {
        #[cfg(feature = "use_openssl")]
        {
            self.sha1.update(data);
            self.sha256.update(data);
        }
        #[cfg(not(feature = "use_openssl"))]
        {
            use sha1::digest::DynDigest;

            self.sha1.update(data);
            self.sha256.update(data);
        }
        Ok(())
    }

    fn finalize(self) -> ([u8; 20], [u8; 32]) {
        #[cfg(feature = "use_openssl")]
        {
            (self.sha1.finish(), self.sha256.finish())
        }
        #[cfg(not(feature = "use_openssl"))]
        {
            use sha2::Digest;
            let sha1_vec = self.sha1.finalize().to_vec();
            let sha2_vec = self.sha256.finalize().to_vec();
            let mut s1 = [0u8; 20];
            let mut s2 = [0u8; 32];
            s1.copy_from_slice(&sha1_vec);
            s2.copy_from_slice(&sha2_vec);
            (s1, s2)
        }
    }
}

impl Seek for HashWriterNull {
    fn seek(&mut self, _pos: SeekFrom) -> impl Future<Output = std::io::Result<u64>> + Send {
        async move {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "HashWriterNull cannot call seek",
            ))
        }
    }
}

impl Write for HashWriterNull {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = std::io::Result<usize>> + Send {
        async move {
            #[cfg(feature = "use_openssl")]
            {
                self.sha1.update(buf);
                self.sha256.update(buf);
                Ok(buf.len())
            }
            #[cfg(not(feature = "use_openssl"))]
            {
                use sha1::digest::DynDigest;
                self.sha1.update(buf);
                self.sha256.update(buf);
                Ok(buf.len())
            }
        }
    }

    fn flush(&mut self) -> impl Future<Output = std::io::Result<()>> + Send {
        async move { Ok(()) }
    }
}
