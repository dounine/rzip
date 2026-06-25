use crate::file::{DataDescriptor, ExtraList, ZipFile};
use crate::hash::{Crc32Reader, HashWriter, HashWriterNull, Hasher};
use crate::zip::{Config, StreamDefault, ZipModel, is_dir};
use binrw::io::read::Read;
use binrw::io::read::ReadExt;
use binrw::io::seek::Seek;
use binrw::io::write::Write;
use binrw::io::{BufReader, BufWriter, ReadBytesCallback};
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, Error};
use miniz_oxide::deflate::CompressionLevel;
use miniz_oxide::inflate::stream::decompress_stream_callback;
use std::string::FromUtf8Error;

// #[binrw]
// #[brw(repr(u16))]
#[derive(Clone, Default, PartialEq)]
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

    fn write_options<'a, 'w, W>(
        &'a self,
        writer: &'w mut W,
        endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send + 'w
    where
        'a: 'w,
        W: Write + Seek + Send,
        Self: Sync + 'a,
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

    fn read_options<'a, 'r, R>(
        reader: &'r mut R,
        endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<Self>> + Send + 'r
    where
        'a: 'r,
        R: Read + Seek + Send,
        Self: Send + 'a,
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
                    return Err(Error::BadMagic(
                        reader.position().await?,
                        "invalid compression method".to_string(),
                    ));
                }
            };
            Ok(value)
        }
    }
}
// #[binrw]
// #[br(import(count:u16,))]
// #[bw()]
#[derive(Clone)]
pub struct Name {
    // #[br(count = count)]
    pub inner: Vec<u8>,
}
impl BinWrite for Name {
    type Args<'a> = ();

    fn write_options<'a, 'w, W>(
        &'a self,
        writer: &'w mut W,
        endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send + 'w
    where
        'a: 'w,
        W: Write + Seek + Send,
        Self: Sync + 'a,
    {
        async move { writer.write_type_args(&self.inner, endian, args).await }
    }
}
impl BinRead for Name {
    type Args<'a> = u16;

    fn read_options<'a, 'r, R>(
        reader: &'r mut R,
        endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<Self>> + Send + 'r
    where
        'a: 'r,
        R: Read + Seek + Send,
        Self: Send + 'a,
    {
        async move {
            let count = args as u64;
            Ok(Name {
                inner: reader.read_type_args(endian, (count, ())).await?,
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
    pub fn into_string(self, _pos: u64) -> BinResult<String> {
        self.clone().try_into().map_err(|e| Error::Err(Box::new(e)))
    }
}
impl TryInto<String> for Name {
    type Error = FromUtf8Error;

    fn try_into(self) -> Result<String, Self::Error> {
        String::from_utf8(self.inner)
    }
}
pub struct Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    pub created_zip_spec: u8,
    pub created_os: u8,
    pub extract_zip_spec: u8,
    pub extract_os: u8,
    // #[bw(calc = 0)]
    pub flags: u16,
    // #[bw(map = |value| if *uncompressed_size == 0 {CompressionMethod::Store}else{value.clone()})]
    pub compression_method: CompressionMethod,
    // #[br(parse_with = compressed_parse,args(&model,&compression_method))]
    // #[bw(if(*model == ZipModel::Bin))]
    pub compressed: bool,
    pub sha_value: Option<([u8; 20], [u8; 32])>,
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
    pub data: Option<T>,
}
impl<T> BinRead for Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    type Args<'a>
        = (
        u16,
        &'a ZipModel,
        &'a T::Config,
        &'a mut ReadBytesCallback<'a>,
    )
    where
        T: 'a;

    fn read_options<'a, 'r, R>(
        reader: &'r mut R,
        endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<Self>> + Send + 'r
    where
        'a: 'r,
        R: Read + Seek + Send,
        Self: Send + 'a,
    {
        async move {
            let (_index, model, config, read_bytes) = args;
            let pos = reader.position().await?;
            let magic: u32 = reader.read_le().await?;
            assert_eq!(magic, 0x02014b50_u32);
            let created_zip_spec: u8 = reader.read_le().await?;
            let created_os: u8 = reader.read_le().await?;
            let extract_zip_spec: u8 = reader.read_le().await?;
            let extract_os: u8 = reader.read_le().await?;
            let flags: u16 = reader.read_le().await?;
            let compression_method: CompressionMethod = reader.read_le().await?;
            let compressed: bool =
                compressed_parse(reader, endian, &model, &compression_method).await?;
            let sha_value = if *model == ZipModel::Bin {
                reader.read_type(endian).await?
            } else {
                None
            };
            let last_modification_time: u16 = reader.read_le().await?;
            let last_modification_date: u16 = reader.read_le().await?;
            let mut crc_32_uncompressed_data: u32 = reader.read_le().await?;
            let mut compressed_size: u32 = reader.read_le().await?;
            let uncompressed_size: u32 = reader.read_le().await?;
            let file_name_length: u16 = reader.read_le().await?;
            let extra_field_length: u16 = reader.read_le().await?;
            let file_comment_length: u16 = reader.read_le().await?;
            let number_of_starts: u16 = reader.read_le().await?;
            let internal_file_attributes: u16 = reader.read_le().await?;
            let _external_file_attributes: u32 = reader.read_le().await?;
            let offset_of_local_file_header: u32 = reader.read_le().await?;
            let file_name: Name = reader.read_le_args(file_name_length).await?;
            let file_name_str = String::from_utf8_lossy(&file_name.inner)
                .to_string()
                .replace("\\", "/");
            let file_name = Name {
                inner: file_name_str.as_bytes().to_vec(),
            };
            let extra_fields: ExtraList = reader.read_le_args(extra_field_length).await?;
            let file_comment: Vec<u8> = reader
                .read_le_args((file_comment_length as u64, ()))
                .await?;
            let mut file: ZipFile = zip_file_parse(
                reader,
                endian,
                &model,
                offset_of_local_file_header,
                uncompressed_size,
            )
            .await?;
            read_bytes(reader.position().await? - pos).await?;
            // reader.seek(SeekFrom::Start(pos))?;
            let data = if is_dir(&file_name.inner) {
                T::from_config(config).await?
            } else {
                if *model == ZipModel::Bin {
                    let length: u64 = reader.read_type(endian).await?;
                    let mut data = reader.take(length);
                    let mut writer = T::from_config(config).await?;
                    binrw::io::copy(&mut data, &mut writer).await?;
                    writer.seek_start().await?;
                    read_bytes(length).await?;
                    writer
                } else {
                    let pos = reader.position().await?;
                    if *model == ZipModel::Parse {
                        reader.set_position(file.data_position).await?;
                    }
                    let config_pos = reader.position().await?;
                    // let mut config = config.clone();
                    // config.compress_size_mut(compressed_size as u64);
                    // config.un_compress_size_mut(uncompressed_size as u64);
                    let (mut data, need_copy) =
                        T::from_link_config(config_pos, compressed_size as u64, config).await?;
                    if need_copy {
                        let mut take_reader = reader.take(compressed_size as u64);
                        let mut buffer = vec![0u8; 1024 * 8];
                        loop {
                            let len = take_reader.read(&mut buffer).await?;
                            if len == 0 {
                                break;
                            }
                            data.write_all(&buffer[..len]).await?;
                            read_bytes(len as u64).await?;
                        }
                    } else {
                        reader.seek_relative(compressed_size as i64).await?;
                        read_bytes(compressed_size as u64).await?;
                    }
                    data.seek_start().await?;
                    let data_descriptor: Option<DataDescriptor> = if file.flags & 0x0008 != 0 {
                        //TODO数据是流式的
                        Some(reader.read_le().await?)
                    } else {
                        None
                    };
                    if let Some(data_descriptor) = &data_descriptor {
                        compressed_size = data_descriptor.compressed_size;
                        crc_32_uncompressed_data = data_descriptor.crc32;
                        file.compressed_size = data_descriptor.compressed_size;
                        file.crc_32_uncompressed_data = crc_32_uncompressed_data;
                    }
                    file.data_descriptor = data_descriptor;
                    if *model == ZipModel::Parse {
                        reader.set_position(pos).await?;
                    }
                    data
                }
            };
            Ok(Self {
                created_zip_spec,
                created_os,
                extract_zip_spec,
                extract_os,
                flags,
                compression_method,
                compressed,
                sha_value,
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
                data: Some(data),
            })
        }
    }
}
impl<T> BinWrite for Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    type Args<'a>
        = (&'a ZipModel,)
    where
        T: 'a;

    fn write_options<'a, 'w, W>(
        &'a self,
        writer: &'w mut W,
        endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send + 'w
    where
        'a: 'w,
        W: Write + Seek + Send,
        Self: Sync + 'a,
    {
        async move {
            // let mut writer = BufWriter::new(writer);
            let (model,) = args;
            writer.write_le(&0x02014b50_u32).await?;
            writer.write_le(&self.created_zip_spec).await?;
            writer.write_le(&self.created_os).await?;
            writer.write_le(&self.extract_zip_spec).await?;
            writer.write_le(&self.extract_os).await?;
            let flags = if is_dir(&self.file_name.inner) {
                0
            } else {
                self.flags
            };
            writer.write_le(&flags).await?; //flags
            writer
                .write_le(if self.uncompressed_size == 0 {
                    &CompressionMethod::Store
                } else {
                    &self.compression_method
                })
                .await?;
            if *model == ZipModel::Bin {
                writer.write_le(&self.compressed).await?;
                writer.write_le(&self.sha_value).await?;
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

            zip_file_writer(writer, endian, &self.file, model, uncompressed_size).await?;
            if let Some(data) = &self.data {
                if self.is_dir() {
                    // writer.flush().await?;
                    return Ok(());
                }
                if *model == ZipModel::Bin {
                    let mut value = data.link().await?;
                    let pos = value.position().await?;
                    value.seek_start().await?;
                    let length: u64 = value.length().await?;
                    writer.write_type(&length, endian).await?;
                    binrw::io::copy(&mut value, writer).await?;
                    value.set_position(pos).await?;
                }
            }
            Ok(())
        }
    }
}

impl<T> Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    pub fn try_clone(&self, config: &T::Config) -> impl Future<Output = BinResult<Self>> + Send {
        Box::pin(async {
            if let Some(data) = &self.data {
                let mut data = data.link().await?;
                let mut new_data = T::from_config(config).await?;
                binrw::io::copy(&mut data, &mut new_data).await?;
                Ok(Self {
                    created_zip_spec: self.created_zip_spec,
                    created_os: self.created_os,
                    extract_zip_spec: self.extract_zip_spec,
                    extract_os: self.extract_os,
                    flags: self.flags,
                    compression_method: self.compression_method.clone(),
                    compressed: self.compressed,
                    sha_value: None,
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
                    data: Some(new_data),
                })
            } else {
                Err(Error::AssertFail("directory data is none".to_string()))
            }
        })
    }
}
impl<T> Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    pub fn is_dir(&self) -> bool {
        crate::zip::is_dir(&self.file_name.inner)
    }
    //     pub fn is_file(name: &Name) -> bool {
    //         !name.inner.ends_with(&[b'/'])
    //     }
}

// #[binrw::parser(reader, endian)]
fn compressed_parse<R>(
    reader: &mut R,
    endian: Endian,
    model: &ZipModel,
    compression_method: &CompressionMethod,
) -> impl Future<Output = BinResult<bool>> + Send
where
    R: Read + Seek + Send,
{
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
    T::Config: Config,
{
    async move {
        let data = T::from_config(config).await?;
        if !is_file {
            return Ok(data);
        }
        let pos = reader.position().await?;
        if *model == ZipModel::Parse {
            reader.set_position(data_position).await?;
        }
        let mut take_reader = reader.take(compressed_size as u64);
        let mut config = config.clone();
        config.compress_size_mut(compressed_size as u64);
        config.un_compress_size_mut(uncompressed_size as u64);
        let mut data = T::from_config(&config).await?;
        binrw::io::copy(&mut take_reader, &mut data).await?;
        data.seek_start().await?;
        // web_sys::console::log_4(
        //     &JsValue::from_str(&name),
        //     &JsValue::from_f64(compressed_size as f64),
        //     &JsValue::from_f64(uncompressed_size as f64),
        //     &JsValue::from_f64(len as f64),
        // );
        if *model == ZipModel::Parse {
            reader.set_position(pos).await?;
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
        let pos = reader.position().await?;
        if *model == ZipModel::Parse {
            reader
                .set_position(offset_of_local_file_header as u64)
                .await?;
        }
        let value = reader
            .read_type_args(endian, (model, uncompressed_size))
            .await?;
        if *model == ZipModel::Parse {
            reader.set_position(pos).await?;
        }
        Ok(value)
    }
}

impl<T> Directory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    pub fn compressed(&self) -> bool {
        self.compressed
    }
    pub fn decompressed_with_writer_callback<'a, W>(
        &mut self,
        writer: &'a mut W,
        callback_fun: &'a mut ReadBytesCallback<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        W: Write + Seek + Send,
    {
        async move {
            if self.compressed() {
                // let (new_data, sha) = {
                if let Some(data) = &mut self.data {
                    data.seek_start().await?;
                    let mut config = data.config().clone();
                    let length = data.length().await?;
                    config.compress_size_mut(length);
                    // let new_data = T::from_config(&config).await?;
                    // let mut hash_writer = HashWriter::new(new_data);
                    decompress_stream_callback(&mut *data, writer, callback_fun)
                        .await
                        .map_err(|e| Error::Err(Box::new(e)))?;
                    // let value = hash_writer.hash();
                    // let mut new_data = hash_writer.into_inner();
                    writer.seek_start().await?;
                    // (new_data, value)
                } else {
                    return Err(Error::AssertFail("compressed data is none".to_string()));
                }
                // };
                // self.sha_value = Some(sha);
                // self.data = Some(new_data);
                self.compressed = false;
            } else {
                if let Some(data) = &mut self.data {
                    data.seek_start().await?;
                    binrw::io::copy(data, writer).await?;
                }
            }
            Ok(())
        }
    }
    pub fn decompressed_callback<'a>(
        &mut self,
        callback_fun: &'a mut ReadBytesCallback<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            if self.compressed() {
                let (new_data, sha) = {
                    if let Some(data) = &mut self.data {
                        data.seek_start().await?;
                        let mut config = data.config().clone();
                        let length = data.length().await?;
                        config.compress_size_mut(length);
                        let new_data = T::from_config(&config).await?;
                        let mut hash_writer = HashWriter::new(new_data);
                        decompress_stream_callback(&mut *data, &mut hash_writer, callback_fun)
                            .await
                            .map_err(|e| Error::Err(Box::new(e)))?;
                        let value = hash_writer.hash();
                        let mut new_data = hash_writer.into_inner();
                        new_data.seek_start().await?;
                        (new_data, value)
                    } else {
                        return Err(Error::AssertFail("compressed data is none".to_string()));
                    }
                };
                self.sha_value = Some(sha);
                self.data = Some(new_data);
                self.compressed = false;
            } else {
                if let Some(data) = &mut self.data {
                    data.seek_start().await?;
                }
            }
            Ok(())
        }
    }
    pub fn decompressed(&mut self) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            self.decompressed_callback(&mut |_| Box::pin(async { Ok(()) }))
                .await?;
            Ok(())
        }
    }
    pub fn copy_data(&mut self) -> impl Future<Output = BinResult<Vec<u8>>> + Send {
        async move {
            if let Some(data) = &mut self.data {
                let pos = data.position().await?;
                let mut bytes = vec![];
                data.read_to_end(&mut bytes).await?;
                data.set_position(pos).await?;
                Ok(bytes)
            } else {
                Err(Error::AssertFail("directory data is none".to_string()))
            }
        }
    }
    pub fn sha_build(&mut self) -> impl Future<Output = BinResult<([u8; 20], [u8; 32])>> + Send {
        async move {
            if let Some(data) = &mut self.data {
                let pos = data.position().await?;
                data.seek_start().await?;
                let mut hasher = HashWriterNull::new();
                binrw::io::copy(&mut *data, &mut hasher).await?;
                data.set_position(pos).await?;
                let sha = hasher.finalize();
                self.sha_value = Some(sha.clone());
                Ok(sha)
            } else {
                Err(Error::AssertFail("directory data is none".to_string()))
            }
        }
    }
    pub fn compress_callback<'a>(
        &'a mut self,
        config: &'a T::Config,
        crc32_computer: bool,
        compression_level: CompressionLevel,
        callback: &'a mut ReadBytesCallback<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            if !self.compressed && self.compression_method == CompressionMethod::Deflate {
                let mut config = config.clone();
                config.compress_size_mut(self.compressed_size as u64);
                let compress_data = {
                    if let Some(mut data) = self.data.take() {
                        data.seek_start().await?;
                        let uncompressed_size = data.length().await?;
                        self.crc_32_uncompressed_data = 0; //crc32 设置为0也能安装，网页可以忽略计算加快速度
                        self.file.crc_32_uncompressed_data = 0;
                        self.uncompressed_size = uncompressed_size as u32;
                        self.file.uncompressed_size = uncompressed_size as u32;
                        let mut config = config.clone();
                        config.compress_size_mut(self.compressed_size as u64);
                        let mut compress_data = T::from_config(&config).await?;
                        let mut crc32_reader = Crc32Reader::new(data);
                        if crc32_computer {
                            crc32_reader.init_crc32();
                        }
                        if uncompressed_size > 0 {
                            miniz_oxide::deflate::stream::compress_stream_callback(
                                &mut crc32_reader,
                                &mut compress_data,
                                compression_level,
                                callback,
                            )
                            .await
                            .map_err(|e| Error::Err(Box::new(e)))?;
                        }
                        self.crc_32_uncompressed_data = crc32_reader.crc32();
                        self.file.crc_32_uncompressed_data = self.crc_32_uncompressed_data;
                        self.compressed_size = compress_data.length().await? as u32;
                        self.file.compressed_size = self.compressed_size;
                        compress_data.seek_start().await?;
                        Some(compress_data)
                    } else {
                        self.crc_32_uncompressed_data = 0;
                        self.file.crc_32_uncompressed_data = 0;
                        None
                    }
                };
                self.data = compress_data;
                self.compressed = true;
            }
            Ok(())
        }
    }
    pub fn compress_to_writer_callback<'a, W>(
        &'a mut self,
        config: &'a T::Config,
        crc32_computer: bool,
        compression_level: CompressionLevel,
        writer: &'a mut W,
        callback: &'a mut ReadBytesCallback<'a>,
    ) -> impl Future<Output = BinResult<Option<(u32, u32)>>> + Send
    where
        W: Write + Seek + Send,
    {
        async move {
            if !self.compressed && self.compression_method == CompressionMethod::Deflate {
                let mut config = config.clone();
                config.compress_size_mut(self.compressed_size as u64);
                let result = if let Some(mut data) = self.data.take() {
                    data.seek_start().await?;
                    let uncompressed_size = data.length().await?;
                    self.crc_32_uncompressed_data = 0;
                    self.file.crc_32_uncompressed_data = 0;
                    self.uncompressed_size = uncompressed_size as u32;
                    self.file.uncompressed_size = uncompressed_size as u32;
                    let mut config = config.clone();
                    config.compress_size_mut(self.compressed_size as u64);
                    let mut crc32_reader = Crc32Reader::new(data);
                    if crc32_computer {
                        crc32_reader.init_crc32();
                    }
                    let pos = writer.position().await?;
                    if uncompressed_size > 0 {
                        miniz_oxide::deflate::stream::compress_stream_callback(
                            &mut crc32_reader,
                            writer,
                            compression_level,
                            callback,
                        )
                        .await
                        .map_err(|e| Error::Err(Box::new(e)))?;
                    }
                    let compress_size = writer.position().await? - pos;
                    Some((crc32_reader.crc32(), compress_size as u32))
                } else {
                    self.crc_32_uncompressed_data = 0;
                    self.file.crc_32_uncompressed_data = 0;
                    None
                };
                self.compressed = true;
                Ok(result)
            } else {
                Ok(None)
            }
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
                Box::pin(async { Ok(()) })
            })
            .await
        }
    }
    pub fn put_data(&mut self, mut stream: T) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            let length = stream.length().await? as u32;
            self.sha_value = None;
            self.compressed_size = length;
            self.uncompressed_size = length;
            self.file.compressed_size = self.compressed_size;
            self.file.uncompressed_size = self.uncompressed_size;
            self.compressed = false;
            self.data = Some(stream);
            Ok(())
        }
    }
}
