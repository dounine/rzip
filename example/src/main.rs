use binrw::BinResult;
use fast_zip::CompressionLevel;
use fast_zip::zip::{Config, FastZip, StreamDefault};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::time::Instant;

#[derive(Debug)]
pub enum MyData {
    File(File),
    Mem(Cursor<Vec<u8>>),
}
#[derive(Default, Clone)]
pub struct MyStreamConfig {
    pub value: bool,
    pub size: Option<u64>,
}
impl Config for MyStreamConfig {
    type Value = bool;

    fn size(&self) -> Option<u64> {
        self.size
    }

    fn size_mut(&mut self, value: u64) {
        self.size = Some(value);
    }

}
impl StreamDefault for MyData {
    type Config = MyStreamConfig;

    fn from_config(config: &Self::Config) -> BinResult<Self> {
        // let v = config.value() as MyStreamConfig;
        // let value = config.value();
        if config.value {}
        Ok(Self::Mem(Cursor::new(vec![])))
    }
}
impl Default for MyData {
    fn default() -> Self {
        Self::Mem(Cursor::new(vec![]))
    }
}
impl Read for MyData {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            MyData::File(v) => v.read(buf),
            MyData::Mem(v) => v.read(buf),
        }
    }
}
impl Write for MyData {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            MyData::File(v) => v.write(buf),
            MyData::Mem(v) => v.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            MyData::File(v) => v.flush(),
            MyData::Mem(v) => v.flush(),
        }
    }
}
impl Seek for MyData {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            MyData::File(v) => v.seek(pos),
            MyData::Mem(v) => v.seek(pos),
        }
    }
}

fn main() {
    let data = Cursor::new(fs::read("./data/mini.ipa".to_string()).unwrap());
    // let mut data = std::fs::File::open("./data/hello.zip".to_string()).unwrap();
    let mut data = MyData::Mem(data);
    // data.read_exact()
    // let mut cursor = Cursor::new(data);
    // let dd = cursor.get_mut();
    let time = Instant::now();

    let mut zip_file: FastZip<MyData> = FastZip::parse(&mut data).unwrap();
    let config = MyStreamConfig::default();
    if let Some(dir) = zip_file.directories.get("hi") {
        let mut new_dir = dir.try_clone(&config).unwrap();
        new_dir.file_name = "".into();
        zip_file.add_directory(new_dir).unwrap();
    }
    // for (a,v) in &mut zip_file.directories.0{
    //    let a = v.clone();
    // }
    dbg!("解析时长", time.elapsed());
    let mut writer = Cursor::new(vec![]);
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
    let mut data = Cursor::new(vec![]);
    let files = vec![
        "Payload/MiniApp.app/embedded.mobileprovision".to_string(),
        "Payload/MiniApp.app/PkgInfo".to_string(),
        "Payload/MiniApp.app/MiniApp".to_string(),
        "Payload/MiniApp.app/Info.plist".to_string(),
        "Payload/MiniApp.app/META-INF/".to_string(),
    ];
    // zip_file.directories.retain(|k, v| files.contains(k));
    let time = Instant::now();
    zip_file.to_bin(&mut data).unwrap();
    dbg!("序列化时长", time.elapsed());
    let time = Instant::now();
    let mut zip_file: FastZip<MyData> = FastZip::from_bin(&mut data).unwrap();
    dbg!("反序列化时长", time.elapsed());
    // let mut zip_file: FastZip<MyData> = FastZip::parse(&mut data).unwrap();
    let time = Instant::now();
    // let config = StreamConfig::default();
    zip_file
        .package(
            &config,
            &mut writer,
            CompressionLevel::DefaultLevel,
            // &mut |total, size, format| println!("write {}", format),
        )
        .unwrap();
    dbg!("压缩时长", time.elapsed());
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open("./data/hello2.zip".to_string())
        .unwrap();
    std::io::copy(&mut writer, &mut file).unwrap();
    // file.write_all(&writer).unwrap();
    // dbg!(zip_file);
    // println!("Hello, world!");
}
