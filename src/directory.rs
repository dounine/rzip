use crate::file::{ExtraList, ZipFile};
use crate::zip::{Magic, ZipModel};
use binrw::{BinResult, binrw};
use std::io::{Read, Seek, SeekFrom, Write};
use std::string::FromUtf8Error;

#[binrw]
#[brw(repr(u16))]
#[derive(Debug, Clone, Default, PartialEq)]
pub enum CompressionMethod {
    #[default]
    Store = 0x0000,
    Shrink = 0x0001,
    Implode = 0x0006,
    Deflate = 0x0008,
    Deflate64 = 0x0009,
    BZIP2 = 0x000C,
    LZMA = 0x000E,
    XZ = 0x005F,
    JPEG = 0x0060,
    WavPack = 0x0061,
    PPMd = 0x0062,
    AES = 0x0063,
}
#[binrw]
#[br(import(count:u16,))]
#[bw()]
#[derive(Debug, Clone)]
pub struct Name {
    #[br(count = count)]
    pub inner: Vec<u8>,
}
impl TryInto<String> for Name {
    type Error = FromUtf8Error;

    fn try_into(self) -> Result<String, Self::Error> {
        String::from_utf8(self.inner)
    }
}

#[binrw]
#[brw(little,import(model:ZipModel,))]
#[derive(Debug, Clone)]
pub struct Directory<T: Read + Write + Seek + Default> {
    #[brw(ignore)]
    pub compressed: bool,
    magic: Magic,
    pub created_zip_spec: u8,
    pub created_os: u8,
    pub extract_zip_spec: u8,
    pub extract_os: u8,
    pub flags: u16,
    pub compression_method: CompressionMethod,
    pub last_modification_time: u16,
    pub last_modification_date: u16,
    pub crc_32_uncompressed_data: u32,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    #[bw(calc = file_name.inner.len() as u16)]
    pub file_name_length: u16,
    #[bw(try_calc = extra_fields.bytes())]
    pub extra_field_length: u16,
    #[bw(calc = file_comment.len() as u16)]
    pub file_comment_length: u16,
    pub number_of_starts: u16,
    pub internal_file_attributes: u16,
    pub external_file_attributes: u32,
    pub offset_of_local_file_header: u32,
    #[br(args(file_name_length,))]
    pub file_name: Name,
    #[br(args(extra_field_length))]
    pub extra_fields: ExtraList,
    #[br(count=file_comment_length)]
    pub file_comment: Vec<u8>,
    #[br(restore_position,seek_before = SeekFrom::Start(offset_of_local_file_header as u64), args(compressed_size,uncompressed_size,crc_32_uncompressed_data,))]
    #[bw(if(model != ZipModel::Package))]
    pub file: ZipFile,
    #[br(restore_position,seek_before = SeekFrom::Start(file.data_position), parse_with = data_init,args(T::default(),file.compressed_size,))]
    #[bw(ignore)]
    pub data: T,
}
#[binrw::parser(reader)]
pub fn data_init<T: Write + Seek + Default>(mut data: T, compressed_size: u32) -> BinResult<T> {
    let mut take_reader = reader.take(compressed_size as u64);
    std::io::copy(&mut take_reader, &mut data)?;
    data.seek(SeekFrom::Start(0))?;
    Ok(data)
}
