use crate::file::{ExtraList, ZipFile};
use crate::util::stream_length;
use crate::zip::ZipModel;
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, Error, binrw};
use miniz_oxide::deflate::CompressionLevel;
use std::fmt::Debug;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
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
impl Name {
    pub fn into_string(self, pos: u64) -> BinResult<String> {
        self.clone().try_into().map_err(|e| Error::Custom {
            pos,
            err: Box::new(e),
        })
    }
}
impl TryInto<String> for Name {
    type Error = FromUtf8Error;

    fn try_into(self) -> Result<String, Self::Error> {
        String::from_utf8(self.inner)
    }
}
#[derive(Debug, Clone)]
pub struct Bool {
    pub value: bool,
}
impl Default for Bool {
    fn default() -> Self {
        Bool { value: true }
    }
}
impl BinWrite for Bool {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        _endian: Endian,
        _args: Self::Args<'_>,
    ) -> BinResult<()> {
        writer.write_all(&[self.value as u8])?;
        Ok(())
    }
}
impl BinRead for Bool {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        _args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let value: u8 = reader.read_type_args(endian, ())?;
        Ok(Bool { value: value != 0 })
    }
}
impl From<bool> for Bool {
    fn from(value: bool) -> Self {
        Bool { value }
    }
}
#[binrw]
#[brw(little,magic = 0x02014b50_u32, import(model:ZipModel,))]
#[derive(Debug, Clone)]
pub struct Directory<T: Read + Write + Seek + Clone + Default> {
    // #[bw(calc = Magic::Directory)]
    // _magic: Magic,
    pub created_zip_spec: u8,
    pub created_os: u8,
    pub extract_zip_spec: u8,
    pub extract_os: u8,
    pub flags: u16,
    pub compression_method: CompressionMethod,
    #[br(parse_with = compressed_parse,args(&model,&compression_method))]
    #[bw(if(model == ZipModel::Bin))]
    pub compressed: Bool,
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
    #[br(parse_with = zip_file_parse,
        args(
            &model,
            offset_of_local_file_header,
            compressed_size,
            uncompressed_size,
            crc_32_uncompressed_data,
        )
    )]
    #[bw(write_with = zip_file_writer,
        args(
            &model,
            *compressed_size,
            *uncompressed_size,
            *crc_32_uncompressed_data,
        )
    )]
    pub file: ZipFile,
    #[br(parse_with = data_parse,args(T::default(),&model,file.data_position,file.compressed_size,))]
    #[bw(write_with = data_write,args(&model,))]
    pub data: T,
}
#[binrw::parser(reader, endian)]
fn compressed_parse(model: &ZipModel, compression_method: &CompressionMethod) -> BinResult<Bool> {
    if *model == ZipModel::Bin {
        return reader.read_type(endian);
    }
    Ok((*compression_method == CompressionMethod::Deflate).into())
}
#[binrw::parser(reader)]
pub fn data_parse<T: Write + Seek + Default>(
    mut data: T,
    model: &ZipModel,
    data_position: u64,
    compressed_size: u32,
) -> BinResult<T> {
    let pos = reader.stream_position()?;
    if *model == ZipModel::Parse {
        reader.seek(SeekFrom::Start(data_position))?;
    }
    let mut take_reader = reader.take(compressed_size as u64);
    std::io::copy(&mut take_reader, &mut data)?;
    data.seek(SeekFrom::Start(0))?;
    if *model == ZipModel::Parse {
        reader.seek(SeekFrom::Start(pos))?;
    }
    Ok(data)
}
#[binrw::writer(writer, endian)]
fn zip_file_writer(
    value: &ZipFile,
    model: &ZipModel,
    compressed_size: u32,
    uncompressed_size: u32,
    crc_32_uncompressed_data: u32,
) -> BinResult<()> {
    if *model == ZipModel::Bin {
        writer.write_type_args(
            value,
            endian,
            (
                model.clone(),
                compressed_size,
                uncompressed_size,
                crc_32_uncompressed_data,
            ),
        )?;
    }
    Ok(())
}
#[binrw::parser(reader, endian)]
fn zip_file_parse(
    model: &ZipModel,
    offset_of_local_file_header: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    crc_32_uncompressed_data: u32,
) -> BinResult<ZipFile> {
    let pos = reader.stream_position()?;
    if *model == ZipModel::Parse {
        reader.seek(SeekFrom::Start(offset_of_local_file_header as u64))?;
    }
    let value = reader.read_type_args(
        endian,
        (
            model.clone(),
            compressed_size,
            uncompressed_size,
            crc_32_uncompressed_data,
        ),
    )?;
    if *model == ZipModel::Parse {
        reader.seek(SeekFrom::Start(pos))?;
    }
    Ok(value)
}
#[binrw::writer(writer)]
fn data_write<T>(value: &T, model: &ZipModel) -> BinResult<()>
where
    T: Read + Write + Seek + Clone + Default,
{
    if *model == ZipModel::Bin {
        let mut value = value.clone();
        value.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut value, writer)?;
    }
    Ok(())
}
impl<T: Read + Write + Seek + Clone + Default> Directory<T> {
    pub fn decompressed(&mut self) -> BinResult<()> {
        self.data.seek(SeekFrom::Start(0))?;
        if self.compressed.value {
            let mut data = Vec::with_capacity(self.uncompressed_size as usize);
            self.data.read_to_end(&mut data)?;
            let un_compress_data =
                miniz_oxide::inflate::decompress_to_vec(&data).map_err(|e| Error::Custom {
                    pos: 0,
                    err: Box::new(e),
                })?;
            let mut new_data = T::default();
            self.uncompressed_size = un_compress_data.len() as u32;
            new_data.write_all(&un_compress_data)?;
            self.data = new_data;
            self.compressed = false.into();
        }
        Ok(())
    }
    pub fn compress(
        &mut self,
        crc32_computer: bool,
        compression_level: &CompressionLevel,
        callback_fun: &mut impl FnMut(usize),
    ) -> BinResult<()> {
        if !self.compressed.value && self.compression_method == CompressionMethod::Deflate {
            let mut data = Cursor::new(Vec::with_capacity(self.uncompressed_size as usize));
            let crc_32_uncompressed_data = if crc32_computer {
                let mut hasher = crc32fast::Hasher::new();
                self.data.seek(SeekFrom::Start(0))?;
                loop {
                    let mut bytes = vec![0_u8; 1024 * 1024];
                    let size = self.data.read(&mut bytes)?;
                    let slice = &bytes[..size];
                    data.write_all(slice)?;
                    hasher.update(slice);
                    if size == 0 {
                        break;
                    }
                }
                hasher.finalize()
            } else {
                self.data.seek(SeekFrom::Start(0))?;
                std::io::copy(&mut self.data, &mut data)?;
                0
            };
            self.crc_32_uncompressed_data = crc_32_uncompressed_data; //crc32 设置为0也能安装，网页可以忽略计算加快速度
            self.file.crc_32_uncompressed_data = crc_32_uncompressed_data;
            let data = data.into_inner();
            let mut compress_data = T::default();
            miniz_oxide::deflate::stream::compress_stream_callback(
                &data,
                &mut compress_data,
                compression_level,
                callback_fun,
            )
            .map_err(|e| Error::Custom {
                pos: 0,
                err: Box::new(e),
            })?;
            self.compressed_size = stream_length(&mut compress_data)? as u32;
            self.file.compressed_size = self.compressed_size;
            self.data = compress_data;
        }
        Ok(())
    }
    pub fn put_data(&mut self, mut stream: T) -> BinResult<()> {
        let length = stream_length(&mut stream)? as u32;
        self.compressed_size = length;
        self.uncompressed_size = length;
        self.file.compressed_size = self.compressed_size;
        self.file.uncompressed_size = self.uncompressed_size;
        self.compressed = false.into();
        self.data = stream;
        Ok(())
    }
}
