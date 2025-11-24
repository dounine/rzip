use crate::directory::{CompressionMethod, Directory, Name};
use crate::extra::Extra;
use crate::file::ZipFile;
use binrw::{BinRead, BinReaderExt, BinResult, BinWriterExt, Endian, Error, binread, binrw};
use indexmap::IndexMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::{Deref, DerefMut};
use crate::util::stream_length;

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
pub struct IndexDirectory<T>(IndexMap<Vec<u8>, Directory<T>>)
where
    T: Read + Write + Seek + Default;


impl<T> DerefMut for IndexDirectory<T>
where
    T: Read + Write + Seek + Default,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl<T> Deref for IndexDirectory<T>
where
    T: Read + Write + Seek + Default,
{
    type Target = IndexMap<Vec<u8>, Directory<T>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
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
            directories.insert(dir.file_name.inner.clone(), dir);
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
    pub fn remove_file(&mut self, file_name: &str) {
        self.directories.swap_remove(file_name.as_bytes());
    }
    pub fn save_file(&mut self, data: T1, file_name: &str) -> BinResult<()> {
        if let Some(dir) = self.directories.get_mut(file_name.as_bytes()) {
            return dir.put_data(data);
        }
        self.add_file(data, file_name)?;
        Ok(())
    }
    pub fn add_directory(&mut self, mut dir: Directory<T1>) -> BinResult<()> {
        if dir.file_name.inner != dir.file.file_name.inner {
            dir.file.file_name = dir.file_name.clone();
        }
        self.directories.insert(dir.file_name.inner.clone(), dir);
        Ok(())
    }
    fn is_binary(data: &[u8]) -> bool {
        let bin_threshold = 0.3;
        let text_chars: Vec<u8> = (0x20..=0x7E) // 可打印 ASCII (空格到 ~)
            .chain(vec![b'\n', b'\r', b'\t', b'\x0B']) // 换行、回车、制表符等
            .collect();
        let non_text_count = data
            .iter()
            .filter(|byte| !text_chars.contains(byte))
            .count();
        let ratio = non_text_count as f32 / data.len() as f32;
        ratio > bin_threshold
    }
    pub fn add_file(&mut self, mut data: T1, file_name: &str) -> BinResult<()> {
        let length = stream_length(&mut data)?;
        let uncompressed_size = length as u32;
        let crc_32_uncompressed_data = 0; //data.crc32_value();
        let compressed_size = uncompressed_size; //data.compress(CompressionLevel::DefaultLevel)? as u32;

        let mut buffer = vec![0u8; std::cmp::min(compressed_size as usize, 1024)];
        data.read_exact(&mut buffer)?;
        data.seek(SeekFrom::Start(0))?;
        let internal_file_attributes = if Self::is_binary(&buffer) { 0 } else { 1 };

        let file_name = Name {
            inner: file_name.as_bytes().to_vec(),
        };
        let directory = Directory {
            compressed: false,
            data,
            created_zip_spec: 0x1E, //3.0
            created_os: 0x03,       //Uninx
            extract_zip_spec: 0x0E, //2.0
            extract_os: 0,          //MS-DOS
            flags: 0,
            compression_method: CompressionMethod::Deflate,
            last_modification_time: 39620,
            last_modification_date: 23170,
            crc_32_uncompressed_data,
            compressed_size,
            uncompressed_size,
            number_of_starts: 0,
            internal_file_attributes,
            external_file_attributes: 2175008768,
            offset_of_local_file_header: 0,
            file_name: file_name.clone(),
            extra_fields: vec![
                Extra::UnixExtendedTimestamp {
                    mtime: Some(1736154637),
                    atime: None,
                    ctime: None,
                },
                Extra::UnixAttrs { uid: 503, gid: 20 },
            ]
            .into(),
            file_comment: vec![],
            file: ZipFile {
                extract_os: 0, //MS-DOS
                compression_method: CompressionMethod::Deflate,
                last_modification_time: 39620,
                last_modification_date: 23170,
                crc_32_uncompressed_data,
                compressed_size,
                uncompressed_size,
                file_name: file_name.clone(),
                extra_fields: vec![
                    Extra::UnixExtendedTimestamp {
                        mtime: Some(1736154637),
                        atime: Some(1736195293),
                        ctime: None,
                    },
                    Extra::UnixAttrs { uid: 503, gid: 20 },
                ]
                .into(),
                data_position: 0,
            },
        };
        self.directories
            .insert(directory.file_name.inner.clone(), directory);
        Ok(())
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
        self.entries = self.directories.len() as u16;
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


#[binrw::parser(reader, endian)]
pub fn parse_eocd_offset() -> BinResult<u64> {
    let max_eocd_size: u64 = u16::MAX as u64 + 22;
    let mut search_size: u64 = 22; //最快的搜索
    let file_size = stream_length(reader)?;
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
