use crate::directory::{CompressionMethod, Name};
use crate::extra::Extra;
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, binrw};
use std::io::{Cursor, Read, Seek, Write};
use std::ops::Deref;

#[binrw]
#[brw(little,magic=0x04034b50_u32,import(compressed_size2:u32,uncompressed_size:u32,crc_32_uncompressed_data:u32))]
#[derive(Debug, Clone)]
pub struct ZipFile {
    #[bw(calc = if file_name.inner.last() == Some(&b'/') { 10 } else { 14 })]
    pub extract_zip_spec: u8,
    pub extract_os: u8,
    #[br(map = |flags:u16| if flags & 0x0008 != 0 { 0 } else { flags })]
    #[bw(calc = 0)]
    pub flags: u16,
    #[br(map = |value| if uncompressed_size == 0 {CompressionMethod::Store}else{value})]
    #[bw(map = |value| if *uncompressed_size == 0 {CompressionMethod::Store}else{value.clone()})]
    pub compression_method: CompressionMethod,
    pub last_modification_time: u16,
    pub last_modification_date: u16,
    // #[br(map = |value| std::cmp::max(crc_32_uncompressed_data,value))]
    // #[bw(map = |value| std::cmp::max(*crc_32_uncompressed_data,*value))]
    pub crc_32_uncompressed_data: u32,
    // #[br(dbg,map = |value| std::cmp::max(compressed_size2,value))]
    // #[bw(map = |value| std::cmp::max(*compressed_size,*value))]
    pub compressed_size: u32,
    // #[br(map = |value| std::cmp::max(uncompressed_size,value))]
    // #[bw(map = |value| std::cmp::max(*uncompressed_size,*value))]
    pub uncompressed_size: u32,
    pub file_name_length: u16,
    #[bw(try_calc = extra_fields.bytes())]
    pub extra_field_length: u16,
    #[br(args(file_name_length,))]
    pub file_name: Name,
    #[br(args(extra_field_length))]
    pub extra_fields: ExtraList,
    // #[br(ignore)]
    // #[bw(ignore)]
    // pub data_descriptor: Option<DataDescriptor>,
    #[br(parse_with = stream_position)]
    #[bw(ignore)]
    pub data_position: u64,
}
#[binrw::parser(reader)]
pub fn stream_position() -> BinResult<u64> {
    reader.stream_position().map_err(|e| binrw::Error::Custom {
        pos: 0,
        err: Box::new(e),
    })
}
#[derive(Debug, Clone)]
pub struct ExtraList(pub Vec<Extra>);
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
        args: Self::Args<'_>,
    ) -> BinResult<()> {
        for extra in &self.0 {
            writer.write_type(extra, endian)?;
        }
        Ok(())
    }
}
impl Deref for ExtraList {
    type Target = Vec<Extra>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
// const ZIP_FILE_HEADER_SIZE: usize = size_of::<Magic>()
//     + size_of::<u16>() * 2
//     + size_of::<CompressionMethod>()
//     + size_of::<u16>() * 2
//     + size_of::<u32>() * 3
//     + size_of::<u16>() * 2;
// #[derive(Debug, Clone)]
// pub struct DataDescriptor {
//     pub crc32: u32,
//     pub compressed_size: u32,
//     pub uncompressed_size: u32,
// }
// impl DataDescriptor {
//     pub fn size() -> usize {
//         4 * 4
//     }
// }
