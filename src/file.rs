use crate::directory::{CompressionMethod, Name};
use crate::extra::Extra;
use crate::zip::ZipModel;
use binrw::io::read::Read;
use binrw::io::seek::Seek;
use binrw::io::write::Write;
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian};
use std::io::Cursor;

// #[binrw]
// #[brw(little, magic = 0x04034b50_u32, import(model:&ZipModel,uncompressed_size:u32))]
#[derive(Debug, Clone)]
pub struct ZipFile {
    // #[bw(calc = if file_name.inner.ends_with(&[b'/']) { 0x0a } else { 0x0e })]
    pub extract_zip_spec: u8,
    pub extract_os: u8,
    // #[br(map = |flags:u16| if flags & 0x0008 != 0 { 0 } else { flags })]
    // #[bw(calc = 0)]
    // pub flags: u16,
    // #[br(map = |value| if uncompressed_size == 0 {CompressionMethod::Store}else{value})]
    // #[bw(map = |value| if *uncompressed_size == 0 {CompressionMethod::Store}else{value.clone()})]
    pub compression_method: CompressionMethod,
    pub last_modification_time: u16,
    pub last_modification_date: u16,
    pub crc_32_uncompressed_data: u32,
    // #[bw(map = |value| if file_name.inner.ends_with(&[b'/']) {0} else {*value})]
    pub compressed_size: u32,
    // #[bw(map = |value| if file_name.inner.ends_with(&[b'/']) {0} else {*value})]
    pub uncompressed_size: u32,
    // #[bw(calc = file_name.inner.len() as u16)]
    pub file_name_length: u16,
    // #[bw(try_calc = extra_fields.bytes())]
    pub extra_field_length: u16,
    // #[br(args(file_name_length, ))]
    pub file_name: Name,
    // #[br(args(extra_field_length))]
    // #[bw(write_with = extra_fields_write, args(file_name.inner.ends_with(&[b'/'])))]
    pub extra_fields: ExtraList,
    // pub data_descriptor: Option<DataDescriptor>,
    // #[br(parse_with = data_position_parse,args(model))]
    // #[bw(if(*model == ZipModel::Bin))]
    pub data_position: u64,
}
impl BinWrite for ZipFile {
    type Args<'a> = (&'a ZipModel, u32);

    fn write_options<W: Write + Seek + Send>(
        &self,
        writer: &mut W,
        _endian: Endian,
        args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        Self: Sync,
    {
        async move {
            let (model, uncompressed_size) = args;
            writer.write_le(&0x04034b50_u32).await?;
            let extract_zip_spec: u8 = if self.file_name.inner.ends_with(&[b'/']) {
                0x0a
            } else {
                0x0e
            };
            writer.write_le(&extract_zip_spec).await?;
            writer.write_le(&self.extract_os).await?;
            writer.write_le(&0_u16).await?;
            let compression_method = if uncompressed_size == 0 {
                &CompressionMethod::Store
            } else {
                &self.compression_method
            };
            writer.write_le(compression_method).await?;
            writer.write_le(&self.last_modification_time).await?;
            writer.write_le(&self.last_modification_date).await?;
            writer.write_le(&self.crc_32_uncompressed_data).await?;
            let compressed_size = if self.file_name.inner.ends_with(&[b'/']) {
                0
            } else {
                self.compressed_size
            };
            writer.write_le(&compressed_size).await?;
            let uncompressed_size = if self.file_name.inner.ends_with(&[b'/']) {
                0
            } else {
                self.uncompressed_size
            };
            writer.write_le(&uncompressed_size).await?;
            let file_name_length = self.file_name.inner.len() as u16;
            writer.write_le(&file_name_length).await?;
            let extra_field_length = self.extra_fields.bytes().await?;
            writer.write_le(&extra_field_length).await?;
            writer.write_le(&self.file_name).await?;
            writer.write_le(&self.extra_fields).await?;
            if *model == ZipModel::Bin {
                writer.write_le(&self.data_position).await?;
            }
            Ok(())
        }
    }
}
impl BinRead for ZipFile {
    type Args<'a> = (&'a ZipModel, u32);

    fn read_options<R: Read + Seek + Send>(
        reader: &mut R,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<Self>> + Send
    where
        Self: Send,
    {
        async move {
            let (model, uncompressed_size) = args;
            let magic: u32 = reader.read_le().await?;
            assert_eq!(magic, 0x04034b50_u32);
            let extract_zip_spec: u8 = reader.read_le().await?;
            let extract_os: u8 = reader.read_le().await?;
            let _flags: u16 = reader.read_le().await?;
            // let flags = if flags & 0x0008 != 0 { 0 } else { flags };
            let mut compression_method: CompressionMethod = reader.read_le().await?;
            if uncompressed_size == 0 {
                compression_method = CompressionMethod::Store;
            }
            let last_modification_time: u16 = reader.read_le().await?;
            let last_modification_date: u16 = reader.read_le().await?;
            let crc_32_uncompressed_data: u32 = reader.read_le().await?;
            let compressed_size: u32 = reader.read_le().await?;
            let uncompressed_size: u32 = reader.read_le().await?;
            let file_name_length: u16 = reader.read_le().await?;
            let extra_field_length: u16 = reader.read_le().await?;
            let file_name: Name = reader.read_le_args(file_name_length).await?;
            let extra_fields: ExtraList = reader.read_le_args(extra_field_length).await?;
            let data_position: u64 = data_position_parse(reader, endian, model).await?;
            Ok(Self {
                extract_zip_spec,
                extract_os,
                // flags,
                compression_method,
                last_modification_time,
                last_modification_date,
                crc_32_uncompressed_data,
                compressed_size,
                uncompressed_size,
                file_name_length,
                extra_field_length,
                file_name,
                extra_fields,
                data_position,
            })
        }
    }
}
// #[binrw::writer(writer)]
pub fn extra_fields_bytes<W: Write + Seek + Send>(
    writer: &mut W,
    extra_field_length: &u16,
    count: u16,
    is_dir: bool,
) -> impl Future<Output = BinResult<()>> + Send {
    async move {
        let mut cursor = Cursor::new(vec![]);
        if is_dir && count == 0 {
            //修复空文件夹没有ext导致无法签名bug
            let value = ExtraList(vec![
                Extra::UnixExtendedTimestamp {
                    mtime: Some(0x66C2AB60_i32),
                    atime: None,
                    ctime: None,
                },
                Extra::UnixAttrs {
                    uid: 0x000001F7_u32,
                    gid: 0x00000014_u32,
                },
            ]);
            cursor.write_le(&value).await?;
            writer.write_le(&(cursor.get_ref().len() as u16)).await?;
        } else {
            writer.write_le(extra_field_length).await?;
        }
        Ok(())
    }
}
// #[binrw::writer(writer)]
pub fn extra_fields_write<W: Write + Seek + Send>(
    writer: &mut W,
    value: &ExtraList,
    is_dir: bool,
) -> impl Future<Output = BinResult<()>> + Send {
    async move {
        if is_dir && value.0.len() == 0 {
            //修复空文件夹没有ext导致无法签名bug
            let value = ExtraList(vec![
                Extra::UnixExtendedTimestamp {
                    mtime: Some(0x66C2AB60_i32),
                    atime: None,
                    ctime: None,
                },
                Extra::UnixAttrs {
                    uid: 0x000001F7_u32,
                    gid: 0x00000014_u32,
                },
            ]);
            writer.write_le(&value).await?;
        } else {
            writer.write_le(value).await?;
        }
        Ok(())
    }
}
// #[binrw::parser(reader, endian)]
pub fn data_position_parse<R: Read + Seek + Send>(
    reader: &mut R,
    endian: Endian,
    model: &ZipModel,
) -> impl Future<Output = BinResult<u64>> + Send {
    async move {
        if *model == ZipModel::Bin {
            return reader.read_type(endian).await;
        }
        reader
            .position()
            .await
            .map_err(|e| binrw::Error::Custom {
                pos: 0,
                err: Box::new(e),
            })
    }
}
#[derive(Debug, Clone)]
pub struct ExtraList(pub Vec<Extra>);
impl From<Vec<Extra>> for ExtraList {
    fn from(value: Vec<Extra>) -> Self {
        ExtraList(value)
    }
}
impl ExtraList {
    pub fn bytes(&self) -> impl Future<Output = BinResult<u16>> + Send {
        async move {
            let mut cursor = Cursor::new(vec![]);
            cursor.write_le(&self.0).await?;
            Ok(cursor.get_ref().len() as u16)
        }
    }
}
impl BinRead for ExtraList {
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
            let bytes = args;
            let mut extra_fields = Vec::new();
            if bytes > 0 {
                let mut total_bytes = 0;
                loop {
                    let position = reader.position().await?;
                    let extra_field: Extra = reader.read_type(endian).await?;
                    extra_fields.push(extra_field);

                    let size = reader.position().await? - position;
                    total_bytes += size;
                    if total_bytes >= bytes as u64 {
                        break;
                    }
                }
            }
            Ok(ExtraList(extra_fields))
        }
    }
}
impl BinWrite for ExtraList {
    type Args<'a> = ();

    fn write_options<W: Write + Seek + Send>(
        &self,
        writer: &mut W,
        endian: Endian,
        _args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        Self: Sync,
    {
        async move {
            for extra in &self.0 {
                writer.write_type(extra, endian).await?;
            }
            Ok(())
        }
    }
}
