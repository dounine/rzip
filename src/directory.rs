use crate::file::{ExtraList, ZipFile};
use crate::util::stream_length;
use crate::zip::{Config, StreamDefault, ZipModel};
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, Error, binrw};
use miniz_oxide::deflate::CompressionLevel;
use miniz_oxide::inflate::stream::decompress_stream_callback;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::cell::RefCell;
use std::fmt::Debug;
use std::io;
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
impl From<String> for Name {
    fn from(value: String) -> Self {
        Self {
            inner: value.into_bytes(),
        }
    }
}
impl From<&str> for Name {
    fn from(value: &str) -> Self {
        Self {
            inner: value.as_bytes().to_vec(),
        }
    }
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
#[brw(little, magic = 0x02014b50_u32)]
#[br(import(model:&ZipModel,config:&T::Config,))]
#[bw(import(model:&ZipModel,))]
#[derive(Debug)]
pub struct Directory<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
    <T::Config as Config>::Value: Default + Clone,
{
    pub created_zip_spec: u8,
    pub created_os: u8,
    pub extract_zip_spec: u8,
    pub extract_os: u8,
    #[bw(calc = 0)]
    pub _flags: u16,
    #[bw(map = |value| if *uncompressed_size == 0 {CompressionMethod::Store}else{value.clone()})]
    pub compression_method: CompressionMethod,
    #[br(parse_with = compressed_parse,args(&model,&compression_method))]
    #[bw(if(*model == ZipModel::Bin))]
    pub compressed: Bool,
    pub last_modification_time: u16,
    pub last_modification_date: u16,
    pub crc_32_uncompressed_data: u32,
    #[bw(map = |value| if self.is_dir() {0} else {*value})]
    pub compressed_size: u32,
    #[bw(map = |value| if self.is_dir() {0} else {*value})]
    pub uncompressed_size: u32,
    #[bw(calc = file_name.inner.len() as u16)]
    pub file_name_length: u16,
    #[bw(try_calc = extra_fields.bytes())]
    pub extra_field_length: u16,
    #[bw(calc = file_comment.len() as u16)]
    pub file_comment_length: u16,
    pub number_of_starts: u16,
    pub internal_file_attributes: u16,
    #[bw(calc =  if self.is_dir() { 0x41ED0010_u32 } else { 0x81A40000_u32 })]
    pub _external_file_attributes: u32,
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
            compressed_size,
            uncompressed_size,
            *crc_32_uncompressed_data,
        )
    )]
    pub file: ZipFile,
    #[br(parse_with = data_parse,args(model,config,Self::is_file(&file_name),file.data_position,compressed_size,uncompressed_size)
    )]
    #[bw(write_with = data_write,args(model,self.is_dir()))]
    pub(crate) data: RefCell<T>,
}
impl<T> Directory<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
    <T::Config as Config>::Value: Default + Clone,
{
    pub fn try_clone(&self, config: &T::Config) -> BinResult<Directory<T>> {
        let mut data = self.data.borrow_mut();
        let size = stream_length(&mut *data)?;
        let pos = data.stream_position()?;
        let mut config = config.clone();
        config.compress_size_mut(size);
        let mut new_data = T::from_config(&config)?;
        std::io::copy(&mut *data, &mut new_data)?;
        data.seek(SeekFrom::Start(pos))?;
        Ok(Self {
            created_zip_spec: self.created_zip_spec,
            created_os: self.created_os,
            extract_zip_spec: self.extract_zip_spec,
            extract_os: self.extract_os,
            compression_method: self.compression_method.clone(),
            compressed: self.compressed.clone(),
            last_modification_time: self.last_modification_time,
            last_modification_date: self.last_modification_date,
            crc_32_uncompressed_data: self.crc_32_uncompressed_data,
            compressed_size: self.compressed_size,
            uncompressed_size: self.uncompressed_size,
            number_of_starts: self.number_of_starts,
            internal_file_attributes: self.internal_file_attributes,
            offset_of_local_file_header: self.offset_of_local_file_header,
            file_name: self.file_name.clone(),
            extra_fields: self.extra_fields.clone(),
            file_comment: self.file_comment.clone(),
            file: self.file.clone(),
            data: RefCell::new(new_data),
        })
    }
}
impl<T> Directory<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
    <T::Config as Config>::Value: Default + Clone,
{
    pub fn is_dir(&self) -> bool {
        self.file_name.inner.ends_with(&[])
    }
    pub fn is_file(name: &Name) -> bool {
        name.inner.ends_with(&[b'/'])
    }
}

#[binrw::parser(reader, endian)]
fn compressed_parse(model: &ZipModel, compression_method: &CompressionMethod) -> BinResult<Bool> {
    if *model == ZipModel::Bin {
        return reader.read_type(endian);
    }
    Ok((*compression_method == CompressionMethod::Deflate).into())
}
#[binrw::parser(reader)]
pub fn data_parse<T>(
    model: &ZipModel,
    config: &T::Config,
    is_dir: bool,
    data_position: u64,
    compressed_size: u32,
    uncompressed_size: u32,
) -> BinResult<RefCell<T>>
where
    T: Write + Seek + StreamDefault,
    T::Config: Config,
    <T::Config as Config>::Value: Default + Clone,
{
    let data = T::from_config(config)?;
    if is_dir {
        return Ok(RefCell::new(data));
    }
    let pos = reader.stream_position()?;
    if *model == ZipModel::Parse {
        reader.seek(SeekFrom::Start(data_position))?;
    }
    let mut take_reader = reader.take(compressed_size as u64);
    let mut config = config.clone();
    config.compress_size_mut(compressed_size as u64);
    config.un_compress_size_mut(uncompressed_size as u64);
    let mut data = T::from_config(&config)?;
    std::io::copy(&mut take_reader, &mut data)?;
    data.seek(SeekFrom::Start(0))?;
    if *model == ZipModel::Parse {
        reader.seek(SeekFrom::Start(pos))?;
    }
    Ok(RefCell::new(data))
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
fn data_write<T>(value: &RefCell<T>, model: &ZipModel, is_dir: bool) -> BinResult<()>
where
    T: Read + Write + Seek + StreamDefault,
{
    if is_dir {
        return Ok(());
    }
    if *model == ZipModel::Bin {
        let mut value = value.borrow_mut();
        let pos = value.stream_position()?;
        value.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut *value, writer)?;
        value.seek(SeekFrom::Start(pos))?;
    }
    Ok(())
}
impl<T> Directory<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
    <T::Config as Config>::Value: Default + Clone,
{
    pub fn data(&self) -> core::cell::Ref<'_, T> {
        self.data.borrow()
    }
    pub fn data_mut(&mut self) -> core::cell::RefMut<'_, T> {
        self.data.borrow_mut()
    }
}
impl<T> Directory<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
    <T::Config as Config>::Value: Default + Clone,
{
    pub fn compressed(&self) -> bool {
        self.compressed.value
    }
    pub fn decompressed_callback(
        &mut self,
        config: &T::Config,
        callback_fun: &mut impl FnMut(usize),
    ) -> BinResult<()> {
        self.data.borrow_mut().seek(SeekFrom::Start(0))?;
        if self.compressed() {
            let mut config = config.clone();
            config.compress_size_mut(stream_length(&mut *self.data.borrow_mut())?);
            let mut new_data = T::from_config(&config)?;
            decompress_stream_callback(&mut *self.data.borrow_mut(), &mut new_data, callback_fun)
                .map_err(|e| Error::Custom {
                pos: 0,
                err: Box::new(e),
            })?;
            new_data.seek(SeekFrom::Start(0))?;
            self.data = new_data.into();
            self.compressed = false.into();
        }
        Ok(())
    }
    pub fn decompressed(&mut self, config: &T::Config) -> BinResult<()> {
        self.decompressed_callback(config, &mut |_| {})
    }
    pub fn copy_data(&mut self) -> BinResult<Vec<u8>> {
        let pos = self.data.borrow_mut().stream_position()?;
        let mut data = vec![];
        self.data.borrow_mut().read_to_end(&mut data)?;
        self.data.borrow_mut().seek(SeekFrom::Start(pos))?;
        Ok(data)
    }
    pub fn sha_value(&mut self) -> BinResult<(Vec<u8>, Vec<u8>)> {
        let pos = self.data.borrow_mut().stream_position()?;
        self.data.borrow_mut().seek(SeekFrom::Start(0))?;
        let mut sha1 = Sha1::new();
        let mut sha256 = Sha256::new();
        let mut multi_writer = HashWriter(&mut sha1, &mut sha256);
        std::io::copy(&mut *self.data.borrow_mut(), &mut multi_writer)?;
        self.data.borrow_mut().seek(SeekFrom::Start(pos))?;
        Ok((sha1.finalize().to_vec(), sha256.finalize().to_vec()))
    }
    pub fn compress_callback(
        &mut self,
        config: &T::Config,
        crc32_computer: bool,
        compression_level: &CompressionLevel,
        callback_fun: &mut impl FnMut(usize),
    ) -> BinResult<()> {
        if !self.compressed.value && self.compression_method == CompressionMethod::Deflate {
            let mut config = config.clone();
            config.compress_size_mut(self.compressed_size as u64);
            self.data.borrow_mut().seek(SeekFrom::Start(0))?;
            let crc_32_uncompressed_data = if crc32_computer {
                let mut hasher = crc32fast::Hasher::new();
                let mut buffer = vec![0u8; 1024 * 1024];
                while let Ok(size) = self.data.borrow_mut().read(&mut buffer) {
                    if size == 0 {
                        break;
                    }
                    let slice = &buffer[..size];
                    hasher.update(slice);
                }
                let value: u32 = hasher.finalize();
                value
            } else {
                0
            };
            self.data.borrow_mut().seek(SeekFrom::Start(0))?;
            let uncompressed_size = stream_length(&mut *self.data.borrow_mut())?;
            self.crc_32_uncompressed_data = crc_32_uncompressed_data; //crc32 设置为0也能安装，网页可以忽略计算加快速度
            self.file.crc_32_uncompressed_data = crc_32_uncompressed_data;
            self.uncompressed_size = uncompressed_size as u32;
            self.file.uncompressed_size = uncompressed_size as u32;
            let mut config = config.clone();
            config.compress_size_mut(self.compressed_size as u64);
            let mut compress_data = T::from_config(&config)?;
            if uncompressed_size > 0 {
                miniz_oxide::deflate::stream::compress_stream_callback(
                    &mut *self.data.borrow_mut(),
                    &mut compress_data,
                    compression_level,
                    callback_fun,
                )
                .map_err(|e| Error::Custom {
                    pos: 0,
                    err: Box::new(e),
                })?;
            }
            self.compressed_size = stream_length(&mut compress_data)? as u32;
            self.file.compressed_size = self.compressed_size;
            compress_data.seek(SeekFrom::Start(0))?;
            self.data = compress_data.into();
        }
        Ok(())
    }
    pub fn compress(
        &mut self,
        config: &T::Config,
        crc32_computer: bool,
        compression_level: &CompressionLevel,
    ) -> BinResult<()> {
        self.compress_callback(config, crc32_computer, compression_level, &mut |_| {})
    }
    pub fn put_data(&mut self, mut stream: T) -> BinResult<()> {
        let length = stream_length(&mut stream)? as u32;
        self.compressed_size = length;
        self.uncompressed_size = length;
        self.file.compressed_size = self.compressed_size;
        self.file.uncompressed_size = self.uncompressed_size;
        self.compressed = false.into();
        self.data = stream.into();
        Ok(())
    }
}
struct HashWriter<A, B>(A, B);

impl<A: Write, B: Write> Write for HashWriter<A, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_all(buf)?;
        self.1.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()?;
        self.1.flush()?;
        Ok(())
    }
}
