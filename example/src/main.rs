use binrw::io::read::{Read, ReadAt};
use binrw::io::seek::Seek;
use binrw::io::write::Write;
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian};
use fast_zip::CompressionLevel;
use fast_zip::zip::{Config, FastZip, StreamDefault};
use std::fmt::{Display, Formatter};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Cursor, SeekFrom};
use std::sync::Arc;
use std::time::Instant;

pub enum MyData {
    File {
        inner: File,
        config: MyStreamConfig,
    },
    Mem {
        inner: Cursor<Vec<u8>>,
        config: MyStreamConfig,
    },
    Shared {
        inner: Arc<dyn ReadAt>,
        offset: u64,
        pos: u64,
        size: u64,
        config: MyStreamConfig,
    },
}
#[derive(Default, Clone)]
pub struct MyStreamConfig {
    pub value: bool,
    pub limit_size: Option<u64>,
    pub compress_size: Option<u64>,
    pub un_compress_size: Option<u64>,
    pub open_files: u16,
    pub source: Option<Arc<dyn ReadAt>>,
}
impl Display for MyStreamConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "MyStreamConfig {{ value: {}, limit: ", self.value)?;

        // 处理 Option<u64> 的显示
        match self.limit_size {
            Some(size) => write!(f, "{}", size)?,
            None => write!(f, "None")?,
        }

        write!(f, ", compress: ")?;
        match self.compress_size {
            Some(size) => write!(f, "{}", size)?,
            None => write!(f, "None")?,
        }

        write!(f, ", uncompress: ")?;
        match self.un_compress_size {
            Some(size) => write!(f, "{}", size)?,
            None => write!(f, "None")?,
        }

        write!(f, ", open_files: {} }}", self.open_files)
    }
}
impl Config for MyStreamConfig {
    // type Value = bool;

    fn compress_size(&self) -> Option<u64> {
        self.compress_size
    }

    fn un_compress_size(&self) -> Option<u64> {
        self.un_compress_size
    }

    fn compress_size_mut(&mut self, value: u64) {
        self.compress_size = Some(value);
    }

    fn un_compress_size_mut(&mut self, value: u64) {
        self.un_compress_size = Some(value);
    }
}
impl BinWrite for MyStreamConfig {
    type Args<'a> = ();

    fn write_options<'a, 'w, W>(
        &'a self,
        writer: &'w mut W,
        endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send + 'w
    where
        'a: 'w,
        W: Write + Seek + Send,
        Self: Sync + 'a,
    {
        async move {
            writer.write_type(&self.value, endian).await?;
            writer.write_type(&self.limit_size, endian).await?;
            writer.write_type(&self.compress_size, endian).await?;
            writer.write_type(&self.un_compress_size, endian).await?;
            writer.write_type(&self.open_files, endian).await?;
            Ok(())
        }
    }
}
impl BinRead for MyStreamConfig {
    type Args<'a> = ();

    fn read_options<'a, 'r, R>(
        reader: &'r mut R,
        endian: Endian,
        _args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<Self>> + Send + 'r
    where
        'a: 'r,
        R: Read + Seek + Send,
        Self: Send + 'a,
    {
        async move {
            Ok(Self {
                value: reader.read_type(endian).await?,
                limit_size: reader.read_type(endian).await?,
                compress_size: reader.read_type(endian).await?,
                un_compress_size: reader.read_type(endian).await?,
                open_files: reader.read_type(endian).await?,
                source: None,
            })
        }
    }
}
impl StreamDefault for MyData {
    type Config = MyStreamConfig;

    // fn from(&self) ->impl Future<Output=BinResult<Self>> + Send {
    //     async move{
    //         MyData::from_config(self.config())
    //     }
    // }

    fn config(&self) -> &Self::Config {
        match self {
            MyData::File { config, .. } => config,
            MyData::Mem { config, .. } => config,
            MyData::Shared { config, .. } => config,
        }
    }

    fn from_config(config: &Self::Config) -> impl Future<Output = BinResult<Self>> + Send {
        async move {
            let mut config = config.clone();
            config.source = None;
            if let (Some(size), Some(limit_size)) = (config.compress_size, config.limit_size) {
                if size > limit_size {
                    let tempfile = tempfile::tempfile()?;
                    return Ok(Self::File {
                        inner: tempfile.into(),
                        config,
                    });
                }
            }
            if let (Some(size), Some(limit_size)) = (config.un_compress_size, config.limit_size) {
                if size > limit_size {
                    let tempfile = tempfile::tempfile()?;
                    return Ok(Self::File {
                        inner: tempfile.into(),
                        config,
                    });
                }
            }
            Ok(Self::Mem {
                inner: Cursor::new(vec![]),
                config,
            })
        }
    }

    fn from_link_config(
        pos: u64,
        size: u64,
        config: &Self::Config,
    ) -> impl Future<Output = BinResult<(Self, bool)>> + Send {
        async move {
            if let Some(source) = &config.source {
                Ok((
                    MyData::Shared {
                        inner: source.clone(),
                        offset: pos,
                        pos: 0,
                        size,
                        config: config.clone(),
                    },
                    false, // false 表示不需要拷贝
                ))
            } else {
                let data = Self::from_config(config).await?;
                Ok((data, true))
            }
        }
    }

    fn from(&self) -> impl Future<Output = BinResult<Self>> + Send {
        MyData::from_config(self.config())
    }

    fn link(&self) -> impl Future<Output = BinResult<Self>> + Send {
        Box::pin(async move { unimplemented!() })
        // match self {
        //     MyData::File { .. } => {}
        //     MyData::Mem { .. } => {}
        //     MyData::Shared { .. } => {}
        // }
    }
}
impl Default for MyData {
    fn default() -> Self {
        Self::Mem {
            inner: Cursor::new(vec![]),
            config: MyStreamConfig::default(),
        }
    }
}
impl Read for MyData {
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            MyData::File { inner, .. } => {
                let value = std::io::Read::read(inner, buf);
                value
            }
            MyData::Mem { inner, .. } => {
                let pos = std::io::Read::read(inner, buf)?;
                Ok(pos)
            }
            MyData::Shared {
                inner,
                offset,
                pos,
                size,
                ..
            } => {
                let remain_len = (*size - *pos) as usize;
                let read_len = buf.len().min(remain_len);
                if read_len == 0 {
                    return Ok(0);
                }
                inner.read_at(&mut buf[..read_len], *offset + *pos)?;
                *pos += read_len as u64;
                Ok(read_len)
            }
        }
    }
    // async fn read(&mut self, buf: &mut [u8]) -> std::io::Error<usize> {
    //     use std::io::Read;
    //     match self {
    //         MyData::File { inner, .. } => {
    //             let value = inner.read(buf).unwrap();
    //             Ok(value)
    //         },
    //         MyData::Mem { inner, .. } => inner.read(buf),
    //     }
    // }

    async fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Write for MyData {
    async fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            MyData::File { inner, .. } => std::io::Write::write(inner, buf),
            MyData::Mem { inner, .. } => std::io::Write::write(inner, buf),
            MyData::Shared { .. } => Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Shared file is read-only",
            )),
        }
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        match self {
            MyData::File { inner, .. } => std::io::Write::flush(inner),
            MyData::Mem { inner, .. } => std::io::Write::flush(inner),
            MyData::Shared { .. } => Ok(()),
        }
    }
}
impl Seek for MyData {
    async fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        use std::io::Seek;
        match self {
            MyData::File { inner, .. } => inner.seek(pos),
            MyData::Mem { inner, .. } => std::io::Seek::seek(inner, pos),
            MyData::Shared {
                offset: _,
                pos: current_pos,
                size,
                ..
            } => {
                let new_pos = match pos {
                    SeekFrom::Start(p) => p,
                    SeekFrom::End(p) => {
                        if p < 0 {
                            if let Some(res) = size.checked_sub((-p) as u64) {
                                res
                            } else {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    "seek before start",
                                ));
                            }
                        } else {
                            *size + p as u64
                        }
                    }
                    SeekFrom::Current(p) => {
                        if p < 0 {
                            if let Some(res) = current_pos.checked_sub((-p) as u64) {
                                res
                            } else {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    "seek before start",
                                ));
                            }
                        } else {
                            *current_pos + p as u64
                        }
                    }
                };
                *current_pos = new_pos;
                Ok(new_pos)
            }
        }
    }
}
#[tokio::main]
async fn main() {
    let data = fs::File::open("./data/SideStore.ipa".to_string()).unwrap();
    // let data = fs::read("./data/SideStore.ipa".to_string()).unwrap();
    // let data = File::open("./data/SideStore.ipa").unwrap();
    let source = Arc::new(data);
    // let file_len =source.len() as u64;
    let file_len = source.metadata().unwrap().len();

    let mut config = MyStreamConfig::default();
    config.source = Some(source.clone());

    // We start with a Shared view of the entire file
    let mut data: MyData = MyData::Shared {
        inner: source.clone(),
        offset: 0,
        pos: 0,
        size: file_len,
        config: config.clone(),
    };
    // data.read_exact()
    // let mut cursor = Cursor::new(data);
    // let dd = cursor.get_mut();
    let time = Instant::now();

    config.limit_size = Some(1024 * 100);
    let mut zip_file: FastZip<MyData> = FastZip::parse_with_callback(&mut data, |total, sum| {
        Box::pin(async move {
            let format = format!("{:.2}%", (sum as f64 / total as f64) * 100.0);
            // println!("process {}", format);
        })
    })
    .await
    .unwrap();
    // for (key,dir) in &mut zip_file.directories.0{
    //     dir.decompressed().await.unwrap();
    // }
    // for (key, dir) in &mut zip_file.directories.0 {
    //     if key == "Payload/SideStore.app/AppIcon60x60@2x.png" {
    //         dir.decompressed().unwrap();
    //         // dir.data_mut().decompressed(&config).unwrap();
    //     }
    //     // let data = &mut *dir.data.borrow_mut();
    //     // let len = data.seek(SeekFrom::End(0)).unwrap();
    //     // let a = &mut *dir.data_mut();
    // }
    // for (key, dir) in &mut zip_file.directories.0 {
    //     if *key == "Payload/Grace.app/Grace" {
    //         dir.decompressed_callback(&config,&mut |_|{}).unwrap();
    //     }
    // }
    // if let Some(dir) = zip_file.directories.get("hi") {
    //     let mut new_dir = dir.try_clone(&config).unwrap();
    //     new_dir.file_name = "".into();
    //     zip_file.add_directory(new_dir).unwrap();
    // }
    // for (a,v) in &mut zip_file.directories.0{
    //    let a = v.clone();
    // }
    dbg!("解析时长", time.elapsed());
    let mut writer = MyData::Mem {
        inner: Cursor::new(vec![]),
        config,
    };
    // let mut data = std::fs::File::open("./data/fsign2.ipa".to_string()).unwrap();
    // zip_file
    //     .add_file(
    //         MyData::File(
    //             fs::File::open(
    //                 "./data/Info.plist"
    //                     .to_string(),
    //             )
    //             .unwrap(),
    //         ),
    //         "Payload/MiniApp.app/Frameworks/MiniUiFramework.framework/Info.plist",
    //     )
    //     .unwrap();
    // zip_file.enable_crc32_computer();
    // let mut file = OpenOptions::new()
    //     .write(true)
    //     .create(true)
    //     .truncate(true)
    //     .open("./data/hello2.zip".to_string())
    //     .unwrap();
    // data.set_position(0);
    // std::io::copy(&mut data, &mut file).unwrap();
    // data.set_position(0);
    // let mut data = Cursor::new(vec![]);
    // let files = vec![
    //     "Payload/MiniApp.app/embedded.mobileprovision".to_string(),
    //     "Payload/MiniApp.app/PkgInfo".to_string(),
    //     "Payload/MiniApp.app/MiniApp".to_string(),
    //     "Payload/MiniApp.app/Info.plist".to_string(),
    //     "Payload/MiniApp.app/META-INF/".to_string(),
    // ];
    // // zip_file.directories.retain(|k, v| files.contains(k));
    // let time = Instant::now();
    // zip_file.to_bin(&mut data).unwrap();
    // dbg!("序列化时长", time.elapsed());
    // let time = Instant::now();
    // let mut zip_file: FastZip<MyData> = FastZip::from_bin(&mut data).unwrap();
    // dbg!("反序列化时长", time.elapsed());
    // // let mut zip_file: FastZip<MyData> = FastZip::parse(&mut data).unwrap();
    // let time = Instant::now();
    // // let config = StreamConfig::default();
    let time = Instant::now();
    // zip_file
    //     .package(
    //         &mut writer,
    //         CompressionLevel::DefaultLevel,
    //         // &mut |total, size, format| println!("write {}", format),
    //     )
    //     .await
    //     .unwrap();
    zip_file
        .package_with_callback(
            &mut writer,
            CompressionLevel::DefaultLevel,
            &mut |total, bytes| {
                Box::pin(async move {
                    // println!("{:.2}%", (bytes as f64 / total as f64) * 100.0)
                })
            },
        )
        .await
        .unwrap();
    writer.seek_start().await.unwrap();
    dbg!("压缩时长", time.elapsed());
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open("./data/hello2.zip".to_string())
        .unwrap();
    binrw::io::copy(&mut writer, &mut file).await.unwrap();
    // file.write_all(&writer).unwrap();
    // dbg!(zip_file);
    // println!("Hello, world!");
}
