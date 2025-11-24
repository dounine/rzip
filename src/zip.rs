use crate::directory::Directory;
use binrw::{
    binread, binrw, BinRead, BinReaderExt, BinResult, BinWriterExt, Endian, Error,
};
use indexmap::IndexMap;
use std::io::{Read, Seek, SeekFrom, Write};

#[binrw]
#[brw(repr(u32))]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ZipModel {
    Parse,
    Package,
    Bin,
}

#[binrw]
#[brw(repr(u32))]
#[derive(Debug, Clone, PartialEq)]
pub enum Magic {
    EoCd = 0x06054b50,
    Directory = 0x02014b50,
    File = 0x04034b50,
}

#[binread]
#[br(little, magic = 0x04034b50_u32, import(model:ZipModel))]
#[derive(Debug, Clone)]
pub struct FastZip<T: Read + Write + Seek + Default> {
    #[br(parse_with = parse_eocd_offset)]
    pub eocd_offset: u64,
    #[br(seek_before = SeekFrom::End(-(eocd_offset as i64)))]
    magic: Magic,
    pub number_of_disk: u16,
    pub directory_starts: u16,
    pub number_of_directory_disk: u16,
    pub entries: u16,
    pub size: u32,
    pub offset: u32,
    pub comment_length: u16,
    #[br(count = comment_length)]
    pub comment: Vec<u8>,
    #[br(seek_before = SeekFrom::Start(offset as u64),args(model, entries,))]
    pub directories: IndexDirectory<T>,
}

#[derive(Debug, Clone)]
pub struct IndexDirectory<T>(IndexMap<String, Directory<T>>)
where
    T: Read + Write + Seek + Default;

impl<T> BinRead for IndexDirectory<T>
where
    T: Read + Write + Seek + Default,
{
    type Args<'a> = (ZipModel, u16);

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let (model, size) = args;
        let mut directories = IndexMap::new();
        for _ in 0..size {
            let dir: Directory<T> = reader.read_type_args(endian, (model.clone(),))?;
            let pos = reader.stream_position()?;
            let file_name = dir
                .file_name
                .clone()
                .try_into()
                .map_err(|e| Error::Custom {
                    pos,
                    err: Box::new(e),
                })?;
            directories.insert(file_name, dir);
        }
        Ok(IndexDirectory(directories))
    }
}

impl<T1> FastZip<T1>
where
    T1: Read + Write + Seek + Default,
{
    pub fn parse<T: Read + Seek>(reader: &mut T) -> BinResult<FastZip<T1>> {
        FastZip::read_le_args(reader, (ZipModel::Parse,))
    }
    pub fn package<W: Write + Seek>(&mut self, writer: &mut W) -> BinResult<()> {
        let mut header = T1::default();
        let mut files_size = 0;
        let mut directors_size = 0;
        for (_, director) in &mut self.directories.0 {
            director.offset_of_local_file_header = files_size as u32;
            let mut directory_writer = T1::default();
            directory_writer.write_le_args(director, (ZipModel::Package,))?;
            directory_writer.seek(SeekFrom::Start(0))?;
            directors_size += std::io::copy(&mut directory_writer, &mut header)?;

            let mut file_writer = T1::default();
            let file = &director.file;
            let mut data = &mut director.data;
            file_writer.write_le(&file)?;
            file_writer.seek(SeekFrom::Start(0))?;
            let file_writer_length = std::io::copy(&mut file_writer, writer)?;
            data.seek(SeekFrom::Start(0))?;
            let file_data_length = std::io::copy(&mut data, writer)?;
            files_size += file_writer_length + file_data_length;
        }
        header.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut header, writer)?;
        self.size = directors_size as u32;
        self.entries = self.directories.0.len() as u16;
        self.number_of_directory_disk = self.entries;
        self.offset = files_size as u32;
        self.write_eocd(writer)?;
        writer.seek(SeekFrom::Start(0))?;
        Ok(())
    }
    fn write_eocd<T: Write + Seek>(&mut self, writer: &mut T) -> BinResult<()> {
        writer.write_le(&self.magic)?;
        writer.write_le(&self.number_of_disk)?;
        writer.write_le(&self.directory_starts)?;
        writer.write_le(&self.number_of_directory_disk)?;
        writer.write_le(&self.entries)?;
        writer.write_le(&self.size)?;
        writer.write_le(&self.offset)?;
        writer.write_le(&self.comment_length)?;
        writer.write_all(&self.comment)?;
        Ok(())
    }
}
fn get_reader_length<R: std::io::Read + std::io::Seek>(reader: &mut R) -> BinResult<u64> {
    // 保存当前位置
    let current_pos = reader.stream_position()?;
    // 移动到末尾获取长度
    let length = reader.seek(SeekFrom::End(0))?;
    // 恢复原始位置
    reader.seek(SeekFrom::Start(current_pos))?;
    Ok(length)
}

#[binrw::parser(reader, endian)]
pub fn parse_eocd_offset() -> BinResult<u64> {
    let max_eocd_size: u64 = u16::MAX as u64 + 22;
    let mut search_size: u64 = 22; //最快的搜索
    let file_size = get_reader_length(reader)?;
    let pos = reader.stream_position()?;

    if file_size < search_size {
        return Err(Error::BadMagic {
            pos,
            found: Box::new("file size le search size, not a zip file"),
        });
    }
    // let eocd_magic: u32 = Magic::EoCd.into();
    loop {
        // 确保搜索范围不超过 EOCD 的最大大小
        search_size = search_size.min(max_eocd_size);
        reader.seek(SeekFrom::End(-(search_size as i64)))?;
        for i in 0..search_size - 3 {
            let pos = reader.stream_position()?;
            // stream.pin()?;
            // let magic: u32 = stream.read_value()?;
            let magic: u32 = BinRead::read_options(reader, endian, ())?;
            reader.seek(SeekFrom::Start(pos))?;
            reader.seek(SeekFrom::Current(1))?;
            // stream.un_pin()?;
            // stream.seek(SeekFrom::Current(1))?;
            if magic == 0x06054b50_u32 {
                return Ok(search_size - i);
            }
            if search_size <= 22 {
                break;
            }
        }
        if search_size >= max_eocd_size {
            break;
        }
        search_size = (search_size * 2).min(file_size);
    }
    Err(Error::BadMagic {
        pos,
        found: Box::new("not a zip file"),
    })
}
