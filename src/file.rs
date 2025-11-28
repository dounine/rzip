use crate::directory::{CompressionMethod, Name};
use crate::extra::Extra;
use crate::zip::ZipModel;
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, binrw};
use std::io::{Cursor, Read, Seek, Write};

#[binrw]
#[brw(little,magic=0x04034b50_u32,import(model:ZipModel,_compressed_size:u32,uncompressed_size:u32,_crc_32_uncompressed_data:u32))]
#[derive(Debug, Clone)]
pub struct ZipFile {
    #[bw(calc = if file_name.inner.last() == Some(&b'/') { 0x0a } else { 0x0e })]
    pub _extract_zip_spec: u8,
    pub extract_os: u8,
    #[br(map = |flags:u16| if flags & 0x0008 != 0 { 0 } else { flags })]
    #[bw(calc = 0)]
    pub _flags: u16,
    #[br(map = |value| if uncompressed_size == 0 {CompressionMethod::Store}else{value})]
    #[bw(map = |value| if *uncompressed_size == 0 {CompressionMethod::Store}else{value.clone()})]
    pub compression_method: CompressionMethod,
    pub last_modification_time: u16,
    pub last_modification_date: u16,
    pub crc_32_uncompressed_data: u32,
    #[bw(map = |value| if file_name.inner.ends_with(&[b'/']) {0} else {*value})]
    pub compressed_size: u32,
    #[bw(map = |value| if file_name.inner.ends_with(&[b'/']) {0} else {*value})]
    pub uncompressed_size: u32,
    #[bw(calc = file_name.inner.len() as u16)]
    pub file_name_length: u16,
    #[bw(try_calc = extra_fields.bytes())]
    // #[bw(write_with = extra_fields_bytes, args(extra_fields.0.len() as u16,file_name.inner.ends_with(&[b'/'])))]
    pub extra_field_length: u16,
    #[br(args(file_name_length,))]
    pub file_name: Name,
    #[br(args(extra_field_length))]
    // #[bw(write_with = extra_fields_write, args(file_name.inner.ends_with(&[b'/'])))]
    pub extra_fields: ExtraList,
    // pub data_descriptor: Option<DataDescriptor>,
    #[br(parse_with = data_position_parse,args(&model))]
    #[bw(if(model == ZipModel::Bin))]
    pub data_position: u64,
}
#[binrw::writer(writer)]
pub fn extra_fields_bytes(extra_field_length: &u16, count: u16, is_dir: bool) -> BinResult<()> {
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
        cursor.write_le(&value)?;
        writer.write_le(&(cursor.get_ref().len() as u16))?;
    } else {
        writer.write_le(extra_field_length)?;
    }
    Ok(())
}
#[binrw::writer(writer)]
pub fn extra_fields_write(
    value: &ExtraList,
    is_dir: bool,
) -> BinResult<()> {
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
        writer.write_le(&value)?;
    } else {
        writer.write_le(value)?;
    }
    Ok(())
}
#[binrw::parser(reader, endian)]
pub fn data_position_parse(model: &ZipModel) -> BinResult<u64> {
    if *model == ZipModel::Bin {
        return reader.read_type(endian);
    }
    reader.stream_position().map_err(|e| binrw::Error::Custom {
        pos: 0,
        err: Box::new(e),
    })
}
#[derive(Debug, Clone)]
pub struct ExtraList(pub Vec<Extra>);
impl From<Vec<Extra>> for ExtraList {
    fn from(value: Vec<Extra>) -> Self {
        ExtraList(value)
    }
}
impl ExtraList {
    pub fn bytes(&self) -> BinResult<u16> {
        let mut cursor = Cursor::new(vec![]);
        cursor.write_type(&self.0, Endian::Little)?;
        Ok(cursor.get_ref().len() as u16)
    }
}
impl BinRead for ExtraList {
    type Args<'a> = (u16,);

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let (bytes,) = args;
        let mut extra_fields = Vec::new();
        if bytes > 0 {
            let mut total_bytes = 0;
            loop {
                let position = reader.stream_position()?;
                let extra_field: Extra = reader.read_type(endian)?;
                extra_fields.push(extra_field);

                let size = reader.stream_position()? - position;
                total_bytes += size;
                if total_bytes >= bytes as u64 {
                    break;
                }
            }
        }
        Ok(ExtraList(extra_fields))
    }
}
impl BinWrite for ExtraList {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        _args: Self::Args<'_>,
    ) -> BinResult<()> {
        for extra in &self.0 {
            writer.write_type(extra, endian)?;
        }
        Ok(())
    }
}
