use std::fmt::{Display, Formatter};
use binrw::BinResult;
use fast_zip::CompressionLevel;
use fast_zip::zip::{Config, FastZip, StreamDefault};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
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
}
#[derive(Default, Clone)]
pub struct MyStreamConfig {
    pub value: bool,
    pub limit_size: Option<u64>,
    pub compress_size: Option<u64>,
    pub un_compress_size: Option<u64>,
    pub open_files: u16,
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
impl StreamDefault for MyData {
    type Config = MyStreamConfig;

    fn from(&self) -> BinResult<Self> {
        MyData::from_config(self.config())
    }

    fn from_config(config: &Self::Config) -> BinResult<Self> {
        if let (Some(size), Some(limit_size)) = (config.compress_size, config.limit_size) {
            if size > limit_size {
                let tempfile = tempfile::tempfile()?;
                return Ok(Self::File {
                    inner: tempfile.into(),
                    config: config.clone(),
                });
            }
        }
        if let (Some(size), Some(limit_size)) = (config.un_compress_size, config.limit_size) {
            if size > limit_size {
                let tempfile = tempfile::tempfile()?;
                return Ok(Self::File {
                    inner: tempfile.into(),
                    config: config.clone(),
                });
            }
        }
        Ok(Self::Mem {
            inner: Cursor::new(vec![]),
            config: config.clone(),
        })
    }

    fn config(&self) -> &Self::Config {
        match self {
            MyData::File { config, .. } => config,
            MyData::Mem { config, .. } => config,
        }
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
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            MyData::File { inner, .. } => inner.read(buf),
            MyData::Mem { inner, .. } => inner.read(buf),
        }
    }
}
impl Write for MyData {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            MyData::File { inner, .. } => inner.write(buf),
            MyData::Mem { inner, .. } => inner.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            MyData::File { inner, .. } => inner.flush(),
            MyData::Mem { inner, .. } => inner.flush(),
        }
    }
}
impl Seek for MyData {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            MyData::File { inner, .. } => inner.seek(pos),
            MyData::Mem { inner, .. } => inner.seek(pos),
        }
    }
}

fn main() {
    let data = fs::File::open("./data/SideStore.ipa".to_string()).unwrap();
    // let mut data = std::fs::File::open("./data/hello.zip".to_string()).unwrap();
    let mut config = MyStreamConfig::default();
    let mut data: MyData = MyData::File {
        inner: data,
        config: config.clone(),
    };
    // data.read_exact()
    // let mut cursor = Cursor::new(data);
    // let dd = cursor.get_mut();
    let time = Instant::now();

    config.limit_size = Some(1024 * 100);
    let mut zip_file: FastZip<MyData> = FastZip::parse(&mut data).unwrap();
    for (key, dir) in &mut zip_file.directories.0 {
        if key == "Payload/SideStore.app/AppIcon60x60@2x.png" {
            dir.decompressed().unwrap();
            // dir.data_mut().decompressed(&config).unwrap();
        }
        // let data = &mut *dir.data.borrow_mut();
        // let len = data.seek(SeekFrom::End(0)).unwrap();
        // let a = &mut *dir.data_mut();
    }
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
    // let mut data = std::fs::File::open("./data/hello.zip".to_string()).unwrap();
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
    // zip_file.disable_crc32_computer();
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
    // zip_file
    //     .package(
    //         &mut writer,
    //         CompressionLevel::DefaultLevel,
    //         // &mut |total, size, format| println!("write {}", format),
    //     )
    //     .unwrap();
    // dbg!("压缩时长", time.elapsed());
    // let mut file = OpenOptions::new()
    //     .write(true)
    //     .create(true)
    //     .truncate(true)
    //     .open("./data/hello2.zip".to_string())
    //     .unwrap();
    // std::io::copy(&mut writer, &mut file).unwrap();
    // file.write_all(&writer).unwrap();
    // dbg!(zip_file);
    // println!("Hello, world!");
}
