use binrw::{BinRead, BinWrite};
use fast_zip::CompressionLevel;
use fast_zip::zip::FastZip;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

#[derive(Debug)]
pub enum MyData {
    File(File),
    Mem(Cursor<Vec<u8>>),
}
impl Clone for MyData {
    fn clone(&self) -> Self {
        match self {
            MyData::File(f) => MyData::File(f.try_clone().unwrap()),
            MyData::Mem(v) => MyData::Mem(Cursor::new(v.get_ref().clone())),
        }
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
    let data = Cursor::new(fs::read("./data/hello.zip".to_string()).unwrap());
    // let mut data = std::fs::File::open("./data/hello.zip".to_string()).unwrap();
    let mut data = MyData::Mem(data);
    // data.read_exact()
    // let mut cursor = Cursor::new(data);
    // let dd = cursor.get_mut();
    let mut zip_file: FastZip<MyData> = FastZip::parse(&mut data).unwrap();
    let mut writer = Cursor::new(vec![]);
    // let mut data = std::fs::File::open("./data/hello.zip".to_string()).unwrap();
    // zip_file
    //     .add_file(
    //         MyData::Mem(Cursor::new(b"hello world hi world nihao nihao nihao hello world hi world nihao nihao nihao hello world hi world nihao nihao nihao hello world hi world nihao nihao nihao".into())),
    //         "hello/nihao.txt",
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
    zip_file.to_bin(&mut data).unwrap();
    let mut zip_file: FastZip<MyData> = FastZip::from_bin(&mut data).unwrap();
    // let mut zip_file: FastZip<MyData> = FastZip::parse(&mut data).unwrap();
    zip_file
        .package(
            &mut writer,
            CompressionLevel::DefaultLevel,
            // &mut |total, size, format| println!("write {}", format),
        )
        .unwrap();
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
