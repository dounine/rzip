use crate::file::{ExtraList, ZipFile};
use crate::util::stream_length;
use crate::zip::{Config, StreamDefault, ZipModel};
use binrw::{
    BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, Error, VecArgs, binrw,
};
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
// #[binwrite]
// #[bw(little, magic = 0x02014b50_u32)]
// #[br(import(model:&ZipModel,config:&T::Config,))]
// #[bw(import(model:&ZipModel,))]
#[derive(Debug)]
pub struct Directory<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
    // <T::Config as Config>::Value: Display + Default + Clone,
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
    pub compressed: Bool,
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
    pub data: RefCell<T>,
}
impl<T> BinRead for Directory<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
    // <T::Config as Config>::Value: Display + Default + Clone,
{
    type Args<'a> = (u16, &'a ZipModel, &'a T::Config);

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let (_index, model, config) = args;
        // if index >= 4 {
        //     return Ok(Self {
        //         created_zip_spec: 0,
        //         created_os: 0,
        //         extract_zip_spec: 0,
        //         extract_os: 0,
        //         compression_method: Default::default(),
        //         compressed: false.into(),
        //         last_modification_time: 0,
        //         last_modification_date: 0,
        //         crc_32_uncompressed_data: 0,
        //         compressed_size: 0,
        //         uncompressed_size: 0,
        //         number_of_starts: 0,
        //         internal_file_attributes: 0,
        //         offset_of_local_file_header: 0,
        //         file_name: Name { inner: vec![] },
        //         extra_fields: ExtraList(vec![]),
        //         file_comment: vec![],
        //         file: ZipFile {
        //             extract_os: 0,
        //             compression_method: Default::default(),
        //             last_modification_time: 0,
        //             last_modification_date: 0,
        //             crc_32_uncompressed_data: 0,
        //             compressed_size: 0,
        //             uncompressed_size: 0,
        //             file_name: Name { inner: vec![] },
        //             extra_fields: ExtraList(vec![]),
        //             data_position: 0,
        //         },
        //         data: RefCell::new(T::from_config(config)?),
        //     });
        // }
        //     if index == 30 {
        //         // web_sys::console::log_3(
        //         //     &JsValue::from_str("长度"),
        //         //     &JsValue::from_f64(0x02014b50_u32 as f64),
        //         //     &JsValue::from_f64(1 as f64),
        //         // );
        //         // let magic: u32 = reader.read_le()?;
        //         // assert_eq!(magic, 0x02014b50_u32);
        //         // web_sys::console::log_3(
        //         //     &JsValue::from_str("长度"),
        //         //     &JsValue::from_f64(0x02014b50_u32 as f64),
        //         //     &JsValue::from_f64(1 as f64),
        //         // );
        //         // let created_zip_spec: u8 = reader.read_le()?;
        //         // let created_os: u8 = reader.read_le()?;
        //         // let extract_zip_spec: u8 = reader.read_le()?;
        //         // let extract_os: u8 = reader.read_le()?;
        //         // let _flags: u16 = reader.read_le()?;
        //         // let compression_method: CompressionMethod = reader.read_le()?;
        //         // let compressed: Bool = compressed_parse(reader, endian, (&model, &compression_method))?;
        //         // let last_modification_time: u16 = reader.read_le()?;
        //         // let last_modification_date: u16 = reader.read_le()?;
        //         // let crc_32_uncompressed_data: u32 = reader.read_le()?;
        //         // let compressed_size: u32 = reader.read_le()?;
        //         // let uncompressed_size: u32 = reader.read_le()?;
        //         // let file_name_length: u16 = reader.read_le()?;
        //         // let extra_field_length: u16 = reader.read_le()?;
        //         // let file_comment_length: u16 = reader.read_le()?;
        //         // let number_of_starts: u16 = reader.read_le()?;
        //         // let internal_file_attributes: u16 = reader.read_le()?;
        //         // let _external_file_attributes: u32 = reader.read_le()?;
        //         // let offset_of_local_file_header: u32 = reader.read_le()?;
        //         // let file_name: Name = reader.read_le_args((file_name_length,))?;
        //         // let extra_fields: ExtraList = reader.read_le_args((extra_field_length,))?;
        //         // let file_comment: Vec<u8> = reader.read_le_args(VecArgs {
        //         //     count: file_comment_length as usize,
        //         //     inner: (),
        //         // })?;
        //         // let file: ZipFile = zip_file_parse(
        //         //     reader,
        //         //     endian,
        //         //     (
        //         //         &model,
        //         //         offset_of_local_file_header,
        //         //         compressed_size,
        //         //         uncompressed_size,
        //         //         crc_32_uncompressed_data,
        //         //     ),
        //         // )?;
        //         // let data: T = if file_name.clone().into_string(0)?
        //         //     == "Payload/SideStore.app/Settings.storyboardc/".to_string()
        //         // {
        //         //     T::from_config(config)?
        //         // } else {
        //         //     // let data: T = {
        //         //
        //         //     let data = T::from_config(config)?;
        //         //     if !Self::is_file(&file_name) {
        //         //         data
        //         //     } else {
        //         //         let pos = reader.stream_position()?;
        //         //         if *model == ZipModel::Parse {
        //         //             reader.seek(SeekFrom::Start(file.data_position))?;
        //         //         }
        //         //         let mut take_reader = reader.take(compressed_size as u64);
        //         //         let mut config = config.clone();
        //         //         config.compress_size_mut(compressed_size as u64);
        //         //         config.un_compress_size_mut(uncompressed_size as u64);
        //         //         let mut data = T::from_config(&config)?;
        //         //         std::io::copy(&mut take_reader, &mut data)?;
        //         //         data.seek(SeekFrom::Start(0))?;
        //         //         // web_sys::console::log_4(
        //         //         //     &JsValue::from_str(&name),
        //         //         //     &JsValue::from_f64(compressed_size as f64),
        //         //         //     &JsValue::from_f64(uncompressed_size as f64),
        //         //         //     &JsValue::from_f64(len as f64),
        //         //         // );
        //         //         if *model == ZipModel::Parse {
        //         //             reader.seek(SeekFrom::Start(pos))?;
        //         //         }
        //         //         data
        //         //     }
        //         // };
        //         // web_sys::console::log_2(
        //         //     &JsValue::from_str("长度"),
        //         //     &JsValue::from_f64(magic as f64),
        //         // );
        //     // let mut bytes = vec![0u8;4];
        //     // reader.read_exact(&mut bytes)?;
        // reader.read_exact(&mut [0u8;4])?;
        let magic: u32 = reader.read_le()?;
        assert_eq!(magic, 0x02014b50_u32);
        let created_zip_spec: u8 = reader.read_le()?;
        let created_os: u8 = reader.read_le()?;
        let extract_zip_spec: u8 = reader.read_le()?;
        let extract_os: u8 = reader.read_le()?;
        let _flags: u16 = reader.read_le()?;
        let compression_method: CompressionMethod = reader.read_le()?;
        let compressed: Bool = compressed_parse(reader, endian, (&model, &compression_method))?;
        let last_modification_time: u16 = reader.read_le()?;
        let last_modification_date: u16 = reader.read_le()?;
        let crc_32_uncompressed_data: u32 = reader.read_le()?;
        let compressed_size: u32 = reader.read_le()?;
        let uncompressed_size: u32 = reader.read_le()?;
        let file_name_length: u16 = reader.read_le()?;
        let extra_field_length: u16 = reader.read_le()?;
        let file_comment_length: u16 = reader.read_le()?;
        let number_of_starts: u16 = reader.read_le()?;
        let internal_file_attributes: u16 = reader.read_le()?;
        let _external_file_attributes: u32 = reader.read_le()?;
        let offset_of_local_file_header: u32 = reader.read_le()?;
        let file_name: Name = reader.read_le_args((file_name_length,))?;
        let extra_fields: ExtraList = reader.read_le_args((extra_field_length,))?;
        let file_comment: Vec<u8> = reader.read_le_args(VecArgs {
            count: file_comment_length as usize,
            inner: (),
        })?;
        // let file = ZipFile {
        //     extract_os: 0,
        //     compression_method: Default::default(),
        //     last_modification_time: 0,
        //     last_modification_date: 0,
        //     crc_32_uncompressed_data: 0,
        //     compressed_size: 0,
        //     uncompressed_size: 0,
        //     file_name: Name { inner: vec![] },
        //     extra_fields: ExtraList(vec![]),
        //     data_position: 0,
        // };
        // let pos = reader.stream_position()?;
        // let len = reader.seek(SeekFrom::End(0))?;
        // reader.seek(SeekFrom::Start(offset_of_local_file_header as u64))?;
        // if index <=4 && len > offset_of_local_file_header as u64 {
        //     // return Err()
        //     let magic: u32 = reader.read_le()?;
        // }
        // let file: ZipFile = reader.read_le_args((model, uncompressed_size))?;
        let file: ZipFile = zip_file_parse(
            reader,
            endian,
            (
                &model,
                offset_of_local_file_header,
                uncompressed_size,
            ),
        )?;
        // reader.seek(SeekFrom::Start(pos))?;
        let data = if !Self::is_file(&file_name) {
            T::from_config(config)?
        } else {
            let pos = reader.stream_position()?;
            if *model == ZipModel::Parse {
                reader.seek(SeekFrom::Start(file.data_position))?;
            }
            let mut take_reader = reader.take(compressed_size as u64);
            let mut config = config.clone();
            config.compress_size_mut(compressed_size as u64);
            config.un_compress_size_mut(uncompressed_size as u64);
            // console::log_2(&JsValue::from_str("come in"),&JsValue::from_str(file_name.clone().into_string(0)?.as_str()));
            let mut data = T::from_config(&config)?;
            std::io::copy(&mut take_reader, &mut data)?;
            // if file_name.clone().into_string(0)? == "Payload/SideStore.app/AppIcon60x60@2x.png" {
            //     let len = data2.seek(SeekFrom::End(0))?;
            //     // console::log_6(
            //     //     &JsValue::from_str("come in"),
            //     //     &JsValue::from_str(format!("{}", data2.config()).as_str()),
            //     //     &JsValue::from_f64(compressed_size as f64),
            //     //     &JsValue::from_f64(uncompressed_size as f64),
            //     //     &JsValue::from_f64(len as f64),
            //     //     &JsValue::from_f64(copy_size as f64),
            //     // );
            //     println!(
            //         "come in {} {} {} {}",
            //         compressed_size, uncompressed_size, len, copy_size,
            //     )
            // }
            data.seek(SeekFrom::Start(0))?;
            if *model == ZipModel::Parse {
                reader.seek(SeekFrom::Start(pos))?;
            }
            data
        };
        // return Ok(Self {
        //     created_zip_spec: 0,
        //     created_os: 0,
        //     extract_zip_spec: 0,
        //     extract_os: 0,
        //     compression_method: Default::default(),
        //     compressed: false.into(),
        //     last_modification_time: 0,
        //     last_modification_date: 0,
        //     crc_32_uncompressed_data: 0,
        //     compressed_size: 0,
        //     uncompressed_size: 0,
        //     number_of_starts: 0,
        //     internal_file_attributes: 0,
        //     offset_of_local_file_header: 0,
        //     file_name: Name { inner: vec![] },
        //     extra_fields: ExtraList(vec![]),
        //     file_comment: vec![],
        //     file: ZipFile {
        //         extract_os: 0,
        //         compression_method: Default::default(),
        //         last_modification_time: 0,
        //         last_modification_date: 0,
        //         crc_32_uncompressed_data: 0,
        //         compressed_size: 0,
        //         uncompressed_size: 0,
        //         file_name: Name { inner: vec![] },
        //         extra_fields: ExtraList(vec![]),
        //         data_position: 0,
        //     },
        //     data: RefCell::new(Box::new(T::from_config(config)?)),
        // });
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
            data: RefCell::new(data),
        })
    }
}
impl<T> BinWrite for Directory<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
    // <T::Config as Config>::Value: Display + Default + Clone,
{
    type Args<'a> = (&'a ZipModel,);

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> BinResult<()> {
        let (model,) = args;
        writer.write_le(&0x02014b50_u32)?;
        writer.write_le(&self.created_zip_spec)?;
        writer.write_le(&self.created_os)?;
        writer.write_le(&self.extract_zip_spec)?;
        writer.write_le(&self.extract_os)?;
        writer.write_le(&0_u16)?; //flags
        writer.write_le(if self.uncompressed_size == 0 {
            &CompressionMethod::Store
        } else {
            &self.compression_method
        })?;
        if *model == ZipModel::Bin {
            writer.write_le(&self.compressed)?;
        }
        writer.write_le(&self.last_modification_time)?;
        writer.write_le(&self.last_modification_date)?;
        writer.write_le(&self.crc_32_uncompressed_data)?;
        let compressed_size = if self.is_dir() {
            0_u32
        } else {
            self.compressed_size
        };
        writer.write_le(&compressed_size)?;
        let uncompressed_size = if self.is_dir() {
            0_u32
        } else {
            self.uncompressed_size
        };
        writer.write_le(&uncompressed_size)?;
        writer.write_le(&(self.file_name.inner.len() as u16))?;
        writer.write_le(&(self.extra_fields.bytes()?))?;
        writer.write_le(&(self.file_comment.len() as u16))?;
        writer.write_le(&self.number_of_starts)?;
        writer.write_le(&self.internal_file_attributes)?;
        writer.write_le(if self.is_dir() {
            &0x41ED0010_u32
        } else {
            &0x81A40000_u32
        })?;
        writer.write_le(&self.offset_of_local_file_header)?;
        writer.write_le(&self.file_name)?;
        writer.write_le(&self.extra_fields)?;
        writer.write_le(&self.file_comment)?;

        zip_file_writer(
            &self.file,
            writer,
            endian,
            (
                &model,
                uncompressed_size,
            ),
        )?;
        data_write(&self.data, writer, endian, (&model, self.is_dir()))?;
        Ok(())
    }
}
impl<T> Directory<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
    // <T::Config as Config>::Value: Display + Default + Clone,
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
    // <T::Config as Config>::Value: Display + Default + Clone,
{
    pub fn is_dir(&self) -> bool {
        self.file_name.inner.ends_with(&[b'/'])
    }
    pub fn is_file(name: &Name) -> bool {
        !name.inner.ends_with(&[b'/'])
    }
}

#[binrw::parser(reader, endian)]
fn compressed_parse(model: &ZipModel, compression_method: &CompressionMethod) -> BinResult<Bool> {
    if *model == ZipModel::Bin {
        return reader.read_type(endian);
    }
    Ok((*compression_method == CompressionMethod::Deflate).into())
}
// #[binrw::parser(reader)]
pub fn data_parse<T, R: Read + Seek>(
    reader: &mut R,
    model: &ZipModel,
    config: &T::Config,
    is_file: bool,
    data_position: u64,
    compressed_size: u32,
    uncompressed_size: u32,
) -> BinResult<T>
where
    T: Read + Write + Seek + StreamDefault,
    T::Config: Config + 'static,
{
    let data = T::from_config(config)?;
    if !is_file {
        return Ok(data);
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
    data.seek(SeekFrom::Start(0))?;
    // web_sys::console::log_4(
    //     &JsValue::from_str(&name),
    //     &JsValue::from_f64(compressed_size as f64),
    //     &JsValue::from_f64(uncompressed_size as f64),
    //     &JsValue::from_f64(len as f64),
    // );
    if *model == ZipModel::Parse {
        reader.seek(SeekFrom::Start(pos))?;
    }
    Ok(data)
}
#[binrw::writer(writer, endian)]
fn zip_file_writer(
    value: &ZipFile,
    model: &ZipModel,
    uncompressed_size: u32,
) -> BinResult<()> {
    if *model == ZipModel::Bin {
        writer.write_type_args(value, endian, (model, uncompressed_size))?;
    }
    Ok(())
}
#[binrw::parser(reader, endian)]
fn zip_file_parse(
    model: &ZipModel,
    offset_of_local_file_header: u32,
    uncompressed_size: u32,
) -> BinResult<ZipFile> {
    let pos = reader.stream_position()?;
    if *model == ZipModel::Parse {
        reader.seek(SeekFrom::Start(offset_of_local_file_header as u64))?;
    }
    let value = reader.read_type_args(endian, (model, uncompressed_size))?;
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
    // <T::Config as Config>::Value: Display + Default + Clone,
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
    // <T::Config as Config>::Value: Display + Default + Clone,
{
    pub fn compressed(&self) -> bool {
        self.compressed.value
    }
    pub fn decompressed_callback(&mut self, callback_fun: &mut impl FnMut(usize)) -> BinResult<()> {
        self.data.borrow_mut().seek(SeekFrom::Start(0))?;
        if self.compressed() {
            let mut config = self.data().config().clone();
            config.compress_size_mut(stream_length(&mut *self.data.borrow_mut())?);
            let mut new_data = T::from_config(&config)?;
            decompress_stream_callback(&mut *self.data.borrow_mut(), &mut new_data, callback_fun)
                .map_err(|e| Error::Custom {
                pos: 0,
                err: Box::new(e),
            })?;
            new_data.seek(SeekFrom::Start(0))?;
            self.data = RefCell::new(new_data);
            self.compressed = false.into();
        }
        Ok(())
    }
    pub fn decompressed(&mut self) -> BinResult<()> {
        self.decompressed_callback(&mut |_| {})
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
            self.data = RefCell::new(compress_data);
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
        self.data = RefCell::new(stream);
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
