use crate::file::{ExtraList, ZipFile};
use crate::util::stream_length;
use crate::zip::{Config, ReadBytesFun, StreamDefault, ZipModel};
use binrw::io::read::Read;
use binrw::io::read::ReadExt;
use binrw::io::seek::Seek;
use binrw::io::write::Write;
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, Error};
use miniz_oxide::deflate::CompressionLevel;
use miniz_oxide::inflate::stream::decompress_stream_callback;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::fmt::Debug;
use std::io;
use std::io::SeekFrom;
use std::string::FromUtf8Error;
use std::sync::Arc;

// #[binrw]
// #[brw(repr(u16))]
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
impl BinWrite for CompressionMethod {
    type Args<'a> = ();

    fn write_options<W: Write + Seek + Send>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        Self: Sync,
    {
        async move {
            let value: u16 = self.clone().into();
            writer.write_type_args::<u16>(&value, endian, args).await?;
            Ok(())
        }
    }
}
impl Into<u16> for CompressionMethod {
    fn into(self) -> u16 {
        match self {
            Self::Store => 0x0000,
            Self::Shrink => 0x0001,
            Self::Implode => 0x0006,
            Self::Deflate => 0x0008,
            Self::Deflate64 => 0x0009,
            Self::BZIP2 => 0x000C,
            Self::LZMA => 0x000E,
            Self::XZ => 0x005F,
            Self::JPEG => 0x0060,
            Self::WavPack => 0x0061,
            Self::PPMd => 0x0062,
            Self::AES => 0x0063,
        }
    }
}
impl BinRead for CompressionMethod {
    type Args<'a> = ();

    fn read_options<R: Read + Seek + Send>(
        reader: &mut R,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<Self>> + Send
    where
        Self: Send,
    {
        async move {
            let result = reader.read_type_args::<u16>(endian, args).await?;
            let value = match result {
                0x0000 => Self::Store,
                0x0001 => Self::Shrink,
                0x0006 => Self::Implode,
                0x0008 => Self::Deflate,
                0x0009 => Self::Deflate64,
                0x000C => Self::BZIP2,
                0x000E => Self::LZMA,
                0x005F => Self::XZ,
                0x0060 => Self::JPEG,
                0x0061 => Self::WavPack,
                0x0062 => Self::PPMd,
                0x0063 => Self::AES,
                _ => {
                    return Err(Error::Custom {
                        pos: reader.stream_position().await?,
                        err: Box::new(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid compression method",
                        )),
                    });
                }
            };
            Ok(value)
        }
    }
}
// #[binrw]
// #[br(import(count:u16,))]
// #[bw()]
#[derive(Debug, Clone)]
pub struct Name {
    // #[br(count = count)]
    pub inner: Vec<u8>,
}
impl BinWrite for Name {
    type Args<'a> = ();

    fn write_options<W: Write + Seek + Send>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        Self: Sync,
    {
        async move { writer.write_type_args(&self.inner, endian, args).await }
    }
}
impl BinRead for Name {
    type Args<'a> = u16;

    fn read_options<R: Read + Seek + Send>(
        reader: &mut R,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<Self>> + Send
    where
        Self: Send,
    {
        async move {
            let count = args as usize;
            Ok(Name {
                inner: reader.read_type_args(endian, count).await?,
            })
        }
    }
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
#[derive(Debug)]
pub struct Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config + 'static,
{
    pub created_zip_spec: u8,
    pub created_os: u8,
    pub extract_zip_spec: u8,
    pub extract_os: u8,
    // #[bw(calc = 0)]
    // pub _flags: u16,
    // #[bw(map = |value| if *uncompressed_size == 0 {CompressionMethod::Store}else{value.clone()})]
    pub compression_method: CompressionMethod,
    // #[br(parse_with = compressed_parse,args(&model,&compression_method))]
    // #[bw(if(*model == ZipModel::Bin))]
    pub compressed: bool,
    pub last_modification_time: u16,
    pub last_modification_date: u16,
    pub crc_32_uncompressed_data: u32,
    // #[bw(map = |value| if self.is_dir() {0} else {*value})]
    pub compressed_size: u32,
    // #[bw(map = |value| if self.is_dir() {0} else {*value})]
    pub uncompressed_size: u32,
    // #[bw(calc = file_name.inner.len() as u16)]
    // pub file_name_length: u16,
    // #[bw(try_calc = extra_fields.bytes())]
    // pub extra_field_length: u16,
    // #[bw(calc = file_comment.len() as u16)]
    // pub file_comment_length: u16,
    pub number_of_starts: u16,
    pub internal_file_attributes: u16,
    // #[bw(calc =  if self.is_dir() { 0x41ED0010_u32 } else { 0x81A40000_u32 })]
    // pub _external_file_attributes: u32,
    pub offset_of_local_file_header: u32,
    // #[br(args(file_name_length,))]
    pub file_name: Name,
    // #[br(args(extra_field_length))]
    pub extra_fields: ExtraList,
    // #[br(count=file_comment_length)]
    pub file_comment: Vec<u8>,
    // #[br(parse_with = zip_file_parse,
    //     args(
    //         &model,
    //         offset_of_local_file_header,
    //         compressed_size,
    //         uncompressed_size,
    //         crc_32_uncompressed_data,
    //     )
    // )]
    // #[bw(write_with = zip_file_writer,
    //     args(
    //         &model,
    //         compressed_size,
    //         uncompressed_size,
    //         *crc_32_uncompressed_data,
    //     )
    // )]
    pub file: ZipFile,
    // #[br(parse_with = data_parse,args(model,config,Self::is_file(&file_name),&file_name,file.data_position,compressed_size,file.compressed_size,uncompressed_size)
    // )]
    // #[bw(write_with = data_write,args(model,self.is_dir()))]
    pub data: Arc<async_lock::Mutex<Option<T>>>,
}
impl<T> BinRead for Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config + 'static,
{
    type Args<'a> = (u16, &'a ZipModel, &'a T::Config, &'a mut ReadBytesFun<'a>);
    fn read_options<R: Read + Seek + Send>(
        reader: &mut R,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<Self>> + Send
    where
        Self: Send,
    {
        async move {
            let (_index, model, config, read_bytes) = args;
            let pos = reader.stream_position().await?;
            let magic: u32 = reader.read_le().await?;
            assert_eq!(magic, 0x02014b50_u32);
            let created_zip_spec: u8 = reader.read_le().await?;
            let created_os: u8 = reader.read_le().await?;
            let extract_zip_spec: u8 = reader.read_le().await?;
            let extract_os: u8 = reader.read_le().await?;
            let _flags: u16 = reader.read_le().await?;
            let compression_method: CompressionMethod = reader.read_le().await?;
            let compressed: bool =
                compressed_parse(reader, endian, &model, &compression_method).await?;
            let last_modification_time: u16 = reader.read_le().await?;
            let last_modification_date: u16 = reader.read_le().await?;
            let crc_32_uncompressed_data: u32 = reader.read_le().await?;
            let compressed_size: u32 = reader.read_le().await?;
            let uncompressed_size: u32 = reader.read_le().await?;
            let file_name_length: u16 = reader.read_le().await?;
            let extra_field_length: u16 = reader.read_le().await?;
            let file_comment_length: u16 = reader.read_le().await?;
            let number_of_starts: u16 = reader.read_le().await?;
            let internal_file_attributes: u16 = reader.read_le().await?;
            let _external_file_attributes: u32 = reader.read_le().await?;
            let offset_of_local_file_header: u32 = reader.read_le().await?;
            let file_name: Name = reader.read_le_args(file_name_length).await?;
            let extra_fields: ExtraList = reader.read_le_args(extra_field_length).await?;
            let file_comment: Vec<u8> = reader.read_le_args(file_comment_length as usize).await?;
            let file: ZipFile = zip_file_parse(
                reader,
                endian,
                &model,
                offset_of_local_file_header,
                uncompressed_size,
            )
            .await?;
            read_bytes(reader.stream_position().await? - pos).await;
            // reader.seek(SeekFrom::Start(pos))?;
            let data = if !Self::is_file(&file_name) {
                T::from_config(config).await?
            } else {
                let pos = reader.stream_position().await?;
                if *model == ZipModel::Parse {
                    reader.seek(SeekFrom::Start(file.data_position)).await?;
                }
                let config_pos = reader.stream_position().await?;
                let mut take_reader = reader.take(compressed_size as u64);
                // let mut config = config.clone();
                // config.compress_size_mut(compressed_size as u64);
                // config.un_compress_size_mut(uncompressed_size as u64);
                let (mut data, need_copy) =
                    T::from_ref_config(config_pos, compressed_size as u64, config).await?;
                if need_copy {
                    let chunk_size = 1024;
                    let mut buffer = vec![0u8; chunk_size];
                    loop {
                        let len = take_reader.read(&mut buffer).await?;
                        if len == 0 {
                            break;
                        }
                        data.write_all(&buffer[..len]).await?;
                        read_bytes(len as u64).await;
                    }
                } else {
                    read_bytes(compressed_size as u64).await;
                }
                data.seek(SeekFrom::Start(0)).await?;
                if *model == ZipModel::Parse {
                    reader.seek(SeekFrom::Start(pos)).await?;
                }
                data
            };
            Ok(Self {
                created_zip_spec,
                created_os,
                extract_zip_spec,
                extract_os,
                compression_method,
                compressed,
                last_modification_time,
                last_modification_date,
                crc_32_uncompressed_data,
                compressed_size,
                uncompressed_size,
                number_of_starts,
                internal_file_attributes,
                offset_of_local_file_header,
                file_name,
                extra_fields,
                file_comment,
                file,
                data: Arc::new(async_lock::Mutex::new(Some(data))),
            })
        }
    }
}
impl<T> BinWrite for Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config + 'static,
{
    type Args<'a> = (&'a ZipModel,);

    fn write_options<W: Write + Seek + Send>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        Self: Sync,
    {
        async move {
            let (model,) = args;
            writer.write_le(&0x02014b50_u32).await?;
            writer.write_le(&self.created_zip_spec).await?;
            writer.write_le(&self.created_os).await?;
            writer.write_le(&self.extract_zip_spec).await?;
            writer.write_le(&self.extract_os).await?;
            writer.write_le(&0_u16).await?; //flags
            writer
                .write_le(if self.uncompressed_size == 0 {
                    &CompressionMethod::Store
                } else {
                    &self.compression_method
                })
                .await?;
            if *model == ZipModel::Bin {
                writer.write_le(&self.compressed).await?;
            }
            writer.write_le(&self.last_modification_time).await?;
            writer.write_le(&self.last_modification_date).await?;
            writer.write_le(&self.crc_32_uncompressed_data).await?;
            let compressed_size = if self.is_dir() {
                0_u32
            } else {
                self.compressed_size
            };
            writer.write_le(&compressed_size).await?;
            let uncompressed_size = if self.is_dir() {
                0_u32
            } else {
                self.uncompressed_size
            };
            writer.write_le(&uncompressed_size).await?;
            writer
                .write_le(&(self.file_name.inner.len() as u16))
                .await?;
            writer.write_le(&(self.extra_fields.bytes().await?)).await?;
            writer.write_le(&(self.file_comment.len() as u16)).await?;
            writer.write_le(&self.number_of_starts).await?;
            writer.write_le(&self.internal_file_attributes).await?;
            writer
                .write_le(if self.is_dir() {
                    &0x41ED0010_u32
                } else {
                    &0x81A40000_u32
                })
                .await?;
            writer.write_le(&self.offset_of_local_file_header).await?;
            writer.write_le(&self.file_name).await?;
            writer.write_le(&self.extra_fields).await?;
            writer.write_le(&self.file_comment).await?;

            zip_file_writer(writer, endian, &self.file, &model, uncompressed_size).await?;
            data_write(writer, self.data.clone(), &model, self.is_dir()).await?;
            Ok(())
        }
    }
}

impl<T> Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config + 'static,
{
    pub fn try_clone(&self, config: &T::Config) -> impl Future<Output = BinResult<Self>> + Send {
        async move {
            let mut data = self.data.lock().await;
            if let Some(data) = &mut *data {
                let mut new_data = T::from_config(config).await?;
                binrw::io::copy(&mut *data, &mut new_data).await?;
                Ok(Self {
                    created_zip_spec: self.created_zip_spec,
                    created_os: self.created_os,
                    extract_zip_spec: self.extract_zip_spec,
                    extract_os: self.extract_os,
                    compression_method: self.compression_method.clone(),
                    compressed: self.compressed,
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
                    data: Arc::new(async_lock::Mutex::new(Some(new_data))),
                })
            } else {
                Err(Error::AssertFail {
                    pos: 0,
                    message: "directory data is none".to_string(),
                })
            }
        }
    }
}
impl<T> Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config + 'static,
    // <T::Config as Config>::Value: Display + Default + Clone,
{
    pub fn is_dir(&self) -> bool {
        self.file_name.inner.ends_with(&[b'/'])
    }
    pub fn is_file(name: &Name) -> bool {
        !name.inner.ends_with(&[b'/'])
    }
}

// #[binrw::parser(reader, endian)]
fn compressed_parse<R: Read + Seek + Send>(
    reader: &mut R,
    endian: Endian,
    model: &ZipModel,
    compression_method: &CompressionMethod,
) -> impl Future<Output = BinResult<bool>> + Send {
    async move {
        if *model == ZipModel::Bin {
            return reader.read_type(endian).await;
        }
        Ok(*compression_method == CompressionMethod::Deflate)
    }
}
// #[binrw::parser(reader)]
pub fn data_parse<T, R: Read + Seek + Send>(
    reader: &mut R,
    model: &ZipModel,
    config: &T::Config,
    is_file: bool,
    data_position: u64,
    compressed_size: u32,
    uncompressed_size: u32,
) -> impl Future<Output = BinResult<T>> + Send
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config + 'static,
{
    async move {
        let data = T::from_config(config).await?;
        if !is_file {
            return Ok(data);
        }
        let pos = reader.stream_position().await?;
        if *model == ZipModel::Parse {
            reader.seek(SeekFrom::Start(data_position)).await?;
        }
        let mut take_reader = reader.take(compressed_size as u64);
        let mut config = config.clone();
        config.compress_size_mut(compressed_size as u64);
        config.un_compress_size_mut(uncompressed_size as u64);
        let mut data = T::from_config(&config).await?;
        binrw::io::copy(&mut take_reader, &mut data).await?;
        data.seek(SeekFrom::Start(0)).await?;
        // web_sys::console::log_4(
        //     &JsValue::from_str(&name),
        //     &JsValue::from_f64(compressed_size as f64),
        //     &JsValue::from_f64(uncompressed_size as f64),
        //     &JsValue::from_f64(len as f64),
        // );
        if *model == ZipModel::Parse {
            reader.seek(SeekFrom::Start(pos)).await?;
        }
        Ok(data)
    }
}
// #[binrw::writer(writer, endian)]
fn zip_file_writer<W: Write + Seek + Send>(
    writer: &mut W,
    endian: Endian,
    value: &ZipFile,
    model: &ZipModel,
    uncompressed_size: u32,
) -> impl Future<Output = BinResult<()>> + Send {
    async move {
        if *model == ZipModel::Bin {
            writer
                .write_type_args(value, endian, (model, uncompressed_size))
                .await?;
        }
        Ok(())
    }
}
// #[binrw::parser(reader, endian)]
fn zip_file_parse<R: Read + Seek + Send>(
    reader: &mut R,
    endian: Endian,
    model: &ZipModel,
    offset_of_local_file_header: u32,
    uncompressed_size: u32,
) -> impl Future<Output = BinResult<ZipFile>> + Send {
    async move {
        let pos = reader.stream_position().await?;
        if *model == ZipModel::Parse {
            reader
                .seek(SeekFrom::Start(offset_of_local_file_header as u64))
                .await?;
        }
        let value = reader
            .read_type_args(endian, (model, uncompressed_size))
            .await?;
        if *model == ZipModel::Parse {
            reader.seek(SeekFrom::Start(pos)).await?;
        }
        Ok(value)
    }
}
// #[binrw::writer(writer)]
fn data_write<T, W: Write + Send>(
    writer: &mut W,
    value: Arc<async_lock::Mutex<Option<T>>>,
    model: &ZipModel,
    is_dir: bool,
) -> impl Future<Output = BinResult<()>> + Send
where
    T: Read + Write + Seek + Send + StreamDefault,
{
    async move {
        if is_dir {
            return Ok(());
        }
        if *model == ZipModel::Bin {
            let mut value = value.lock().await;
            if let Some(value) = &mut *value {
                let pos = value.stream_position().await?;
                value.seek(SeekFrom::Start(0)).await?;
                binrw::io::copy(&mut *value, writer).await?;
                value.seek(SeekFrom::Start(pos)).await?;
            }
        }
        Ok(())
    }
}
// impl<T> Directory<T>
// where
//     T: Read + Write + Seek + Send + StreamDefault,
//     T::Config: Config + 'static,
//     // <T::Config as Config>::Value: Display + Default + Clone,
// {
//     pub fn data(&self) -> core::cell::Ref<'_, T> {
//         self.data.borrow()
//     }
//     pub fn data_mut(&mut self) -> core::cell::RefMut<'_, T> {
//         self.data.borrow_mut()
//     }
// }
impl<T> Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config + 'static,
    // <T::Config as Config>::Value: Display + Default + Clone,
{
    pub fn compressed(&self) -> bool {
        self.compressed
    }
    pub fn decompressed_callback<'a>(
        &mut self,
        callback_fun: &'a mut ReadBytesFun<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            if self.compressed() {
                let new_data = {
                    let mut data = self.data.lock().await;
                    if let Some(data) = &mut *data {
                        data.seek(SeekFrom::Start(0)).await?;
                        let mut config = data.config().clone();
                        let length = stream_length(&mut *data).await?;
                        config.compress_size_mut(length);
                        let mut new_data = T::from_config(&config).await?;
                        decompress_stream_callback(&mut *data, &mut new_data, callback_fun)
                            .await
                            .map_err(|e| Error::Custom {
                                pos: 0,
                                err: Box::new(e),
                            })?;
                        new_data.seek(SeekFrom::Start(0)).await?;
                        new_data
                    } else {
                        return Err(Error::AssertFail {
                            pos: 0,
                            message: "compressed data is none".to_string(),
                        });
                    }
                };
                self.data = Arc::new(async_lock::Mutex::new(Some(new_data)));
                self.compressed = false;
            }
            Ok(())
        }
    }
    pub fn decompressed(&mut self) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            self.decompressed_callback(&mut |_| Box::pin(async {}))
                .await?;
            Ok(())
        }
    }
    pub fn copy_data(&mut self) -> impl Future<Output = BinResult<Vec<u8>>> + Send {
        async move {
            let mut data = self.data.lock().await;
            if let Some(data) = &mut *data {
                let pos = data.stream_position().await?;
                let mut bytes = vec![];
                data.read_to_end(&mut bytes).await?;
                data.seek(SeekFrom::Start(pos)).await?;
                Ok(bytes)
            } else {
                Err(Error::AssertFail {
                    pos: 0,
                    message: "directory data is none".to_string(),
                })
            }
        }
    }
    pub fn sha_value(&mut self) -> impl Future<Output = BinResult<(Vec<u8>, Vec<u8>)>> + Send {
        async move {
            let mut data = self.data.lock().await;
            if let Some(data) = &mut *data {
                let pos = data.stream_position().await?;
                data.seek(SeekFrom::Start(0)).await?;
                let mut multi_writer = HashWriter::new();
                binrw::io::copy(&mut *data, &mut multi_writer).await?;
                data.seek(SeekFrom::Start(pos)).await?;
                Ok(multi_writer.hash())
            } else {
                Err(Error::AssertFail {
                    pos: 0,
                    message: "directory data is none".to_string(),
                })
            }
        }
    }
    pub fn compress_callback<'a>(
        &'a mut self,
        config: &'a T::Config,
        crc32_computer: bool,
        compression_level: CompressionLevel,
        callback_fun: &'a mut ReadBytesFun<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            if !self.compressed && self.compression_method == CompressionMethod::Deflate {
                let mut config = config.clone();
                config.compress_size_mut(self.compressed_size as u64);
                let compress_data = {
                    let mut data = self.data.lock().await;
                    let crc_32_uncompressed_data = if let Some(data) = &mut *data {
                        data.seek(SeekFrom::Start(0)).await?;
                        let crc_32_uncompressed_data = if crc32_computer {
                            let mut hasher = crc32fast::Hasher::new();
                            let mut buffer = vec![0u8; 1024 * 1024];
                            while let Ok(size) = data.read(&mut buffer).await {
                                if size == 0 {
                                    break;
                                }
                                hasher.update(&buffer[..size]);
                            }
                            let value: u32 = hasher.finalize();
                            value
                        } else {
                            0
                        };
                        crc_32_uncompressed_data
                    } else {
                        0
                    };
                    if let Some(data) = &mut *data {
                        data.seek(SeekFrom::Start(0)).await?;
                        let uncompressed_size = stream_length(data).await?;
                        self.crc_32_uncompressed_data = crc_32_uncompressed_data; //crc32 设置为0也能安装，网页可以忽略计算加快速度
                        self.file.crc_32_uncompressed_data = crc_32_uncompressed_data;
                        self.uncompressed_size = uncompressed_size as u32;
                        self.file.uncompressed_size = uncompressed_size as u32;
                        let mut config = config.clone();
                        config.compress_size_mut(self.compressed_size as u64);
                        let mut compress_data = T::from_config(&config).await?;
                        if uncompressed_size > 0 {
                            miniz_oxide::deflate::stream::compress_stream_callback(
                                data,
                                &mut compress_data,
                                compression_level,
                                callback_fun,
                            )
                            .await
                            .map_err(|e| Error::Custom {
                                pos: 0,
                                err: Box::new(e),
                            })?;
                        }
                        self.compressed_size = stream_length(&mut compress_data).await? as u32;
                        self.file.compressed_size = self.compressed_size;
                        compress_data.seek(SeekFrom::Start(0)).await?;
                        Some(compress_data)
                    } else {
                        None
                    }
                };
                self.data = Arc::new(async_lock::Mutex::new(compress_data));
            }
            Ok(())
        }
    }
    pub fn compress(
        &mut self,
        config: &T::Config,
        crc32_computer: bool,
        compression_level: CompressionLevel,
    ) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            self.compress_callback(config, crc32_computer, compression_level, &mut |_| {
                Box::pin(async {})
            })
            .await
        }
    }
    pub fn put_data(&mut self, mut stream: T) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            let length = stream_length(&mut stream).await? as u32;
            self.compressed_size = length;
            self.uncompressed_size = length;
            self.file.compressed_size = self.compressed_size;
            self.file.uncompressed_size = self.uncompressed_size;
            self.compressed = false;
            self.data = Arc::new(async_lock::Mutex::new(Some(stream)));
            Ok(())
        }
    }
}
struct HashWriter(Sha1, Sha256);
impl HashWriter {
    pub fn new() -> Self {
        HashWriter(Sha1::new(), Sha256::new())
    }
    pub fn hash(self) -> (Vec<u8>, Vec<u8>) {
        (self.0.finalize().to_vec(), self.1.finalize().to_vec())
    }
}
impl Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<usize>> + Send {
        async move {
            std::io::Write::write(&mut self.0, buf)?;
            let size = std::io::Write::write(&mut self.1, buf)?;
            Ok(size)
        }
    }

    fn flush(&mut self) -> impl Future<Output = io::Result<()>> + Send {
        async move {
            std::io::Write::flush(&mut self.0)?;
            std::io::Write::flush(&mut self.1)?;
            Ok(())
        }
    }
}
