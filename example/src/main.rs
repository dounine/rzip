use binrw::{BinRead, BinResult, BinWrite, Endian};
use fast_zip::zip::{FastZip, ZipModel};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::SeekFrom::Current;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

#[derive(Debug)]
pub enum MyData {
    File(File),
    Mem(Cursor<Vec<u8>>),
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
    let mut writer= Cursor::new(vec![]);
    // let mut data = std::fs::File::open("./data/hello.zip".to_string()).unwrap();
    zip_file.package(&mut writer).unwrap();
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
