use crate::directory::{CompressionMethod, Directory, Name};
use crate::file::{ExtraList, ZipFile};
use binrw::io::read::Read;
use binrw::io::seek::Seek;
use binrw::io::write::Write;
use binrw::io::{BufReader, ReadBytesCallback};
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, Error};
use indexmap::IndexMap;
use std::collections::HashSet;
use std::io::SeekFrom;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;

pub trait Config: Sync + Send + Clone + Default {
    // type Value;
    fn compress_size(&self) -> u64;
    fn un_compress_size(&self) -> u64;
    fn compress_size_mut(&mut self, value: u64);
    fn un_compress_size_mut(&mut self, value: u64);
    fn temp_dir(&self) -> Option<std::path::PathBuf>;
}

pub trait StreamDefault: Sized + Sync {
    type Config;
    fn from(&self) -> impl Future<Output = BinResult<Self>> + Send;
    fn from_config(config: &Self::Config) -> impl Future<Output = BinResult<Self>> + Send;
    fn from_link_config(
        _pos: u64,
        _size: u64,
        config: &Self::Config,
    ) -> impl Future<Output = BinResult<(Self, bool)>> + Send {
        let data = Self::from_config(config);
        async move { Ok((data.await?, true)) }
    }

    fn config(&self) -> &Self::Config;

    fn link(&self) -> impl Future<Output = BinResult<Self>> + Send;
}

// #[binrw]
// #[brw(repr(u8))]
#[derive(Default, Clone, Eq, PartialEq)]
pub enum ZipModel {
    #[default]
    Parse,
    Bin,
}
impl BinWrite for ZipModel {
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
            let value: u8 = match self {
                Self::Parse => 0x00,
                Self::Bin => 0x01,
            };
            writer.write_type_args(&value, endian, args).await?;
            Ok(())
        }
    }
}
impl BinRead for ZipModel {
    type Args<'a> = ();

    fn read_options<'a, 'r, R>(
        reader: &'r mut R,
        endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<Self>> + Send + 'r
    where
        'a: 'r,
        R: Read + Seek + Send,
    {
        async move {
            let value: u8 = reader.read_type_args(endian, args).await?;
            let model = match value {
                0x00 => Self::Parse,
                0x02 => Self::Bin,
                _ => {
                    let pos = reader.position().await?;
                    return Err(Error::BadMagic(
                        pos,
                        format!("magic {} not match for ZipModel", value),
                    ));
                }
            };
            Ok(model)
        }
    }
}
// #[binrw]
// #[brw(repr(u32))]
#[derive(Clone, PartialEq)]
pub enum Magic {
    EoCd = 0x06054b50,
    Directory = 0x02014b50,
    File = 0x04034b50,
}
impl BinRead for Magic {
    type Args<'a> = ();

    fn read_options<'a, 'r, R>(
        reader: &'r mut R,
        endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<Self>> + Send + 'r
    where
        'a: 'r,
        R: Read + Seek + Send,
    {
        async move {
            let value: u32 = reader.read_type_args(endian, args).await?;
            let value = match value {
                0x06054b50 => Self::EoCd,
                0x02014b50 => Self::Directory,
                0x04034b50 => Self::File,
                _ => {
                    let pos = reader.position().await?;
                    return Err(Error::BadMagic(pos, format!("magic {} not match", value)));
                }
            };
            Ok(value)
        }
    }
}
impl BinWrite for Magic {
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
            let value: u32 = match self {
                Magic::EoCd => 0x06054b50,
                Magic::Directory => 0x02014b50,
                Magic::File => 0x04034b50,
            };
            writer.write_type_args(&value, endian, args).await?;
            Ok(())
        }
    }
}
impl Default for Magic {
    fn default() -> Self {
        Self::EoCd
    }
}

// #[binrw::binwrite]
// #[br(little, magic = 0x04034b50_u32, import(model:ZipModel,c:&T::Config))]
// #[bw(little, magic = 0x04034b50_u32, import(model:ZipModel))]
pub struct FastZip<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    // #[br(calc = c.clone())]
    // #[bw(ignore)]
    pub config: T::Config,
    // #[bw(if(model == ZipModel::Bin))]
    pub crc32_computer: bool,
    // #[br(parse_with = parse_eocd_offset,args(model.clone(),))]
    // #[bw(ignore)]
    pub eocd_offset: u64,
    // #[br(if(model==ZipModel::Parse),seek_before = SeekFrom::End(-(eocd_offset as i64)))]
    // #[bw(if(model==ZipModel::Parse))]
    magic: Magic,
    pub number_of_disk: u16,
    pub directory_starts: u16,
    pub number_of_directory_disk: u16,
    // #[bw(calc = directories.len() as u16)]
    pub entries: u16,
    pub size: u32,
    pub offset: u32,
    // #[bw(calc = comment.len() as u16)]
    pub comment_length: u16,
    // #[br(count = comment_length)]
    pub comment: Vec<u8>,
    // #[br(seek_before = if model == ZipModel::Parse {
    //         SeekFrom::Start(offset as u64)
    //     } else {
    //         SeekFrom::Current(0)
    //     },args(&model,&config, entries,)
    // )]
    // #[bw(if(model == ZipModel::Bin),args(&model,))]
    pub directories: IndexDirectory<T>,
}
impl<T> BinWrite for FastZip<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    type Args<'a>
        = &'a ZipModel
    where
        T: 'a;

    fn write_options<'a, 'w, W>(
        &'a self,
        writer: &'w mut W,
        _endian: Endian,
        args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send + 'w
    where
        'a: 'w,
        W: Write + Seek + Send,
        Self: Sync + 'a,
    {
        async move {
            let model = args;
            writer.write_le(&0x04034b50_u32).await?;
            if *model == ZipModel::Bin {
                writer.write_le(&self.crc32_computer).await?;
            }
            if *model == ZipModel::Parse {
                writer.write_le(&self.magic).await?;
            }
            if *model == ZipModel::Bin {
                writer.write_le(&self.eocd_offset).await?;
            }
            writer.write_le(&self.magic).await?;
            writer.write_le(&self.number_of_disk).await?;
            writer.write_le(&self.directory_starts).await?;
            writer.write_le(&self.number_of_directory_disk).await?;
            writer.write_le(&(self.directories.len() as u16)).await?;
            writer.write_le(&self.size).await?;
            writer.write_le(&self.offset).await?;
            writer.write_le(&(self.comment.len() as u16)).await?;
            writer.write_le(&self.comment).await?;
            if *model == ZipModel::Bin {
                writer.write_le_args(&self.directories, (model,)).await?;
            }
            Ok(())
        }
    }
}
// pub type ReadBytesFun<'a> = dyn FnMut(u64) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'a;
impl<T> BinRead for FastZip<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    type Args<'a>
        = (&'a ZipModel, &'a T::Config, &'a mut ReadBytesCallback<'a>)
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
        Self: 'a,
    {
        async move {
            let (model, config, read_bytes) = args;
            let pos = reader.position().await?;
            let magic: u32 = reader.read_le().await?;
            assert_eq!(magic, 0x04034b50_u32);
            let crc32_computer = if *model == ZipModel::Bin {
                reader.read_le::<bool>().await?
            } else {
                false
            };
            let eocd_offset = if *model == ZipModel::Bin {
                reader.read_le::<u64>().await?
            } else {
                let eocd_offset = parse_eocd_offset(reader, endian, model).await?;
                reader.seek(SeekFrom::End(-(eocd_offset as i64))).await?;
                eocd_offset
            };
            let magic: Magic = reader.read_le().await?;
            let number_of_disk: u16 = reader.read_le().await?;
            let directory_starts: u16 = reader.read_le().await?;
            let number_of_directory_disk: u16 = reader.read_le().await?;
            // #[bw(calc = directories.len() as u16)]
            let entries: u16 = reader.read_le().await?;
            let size: u32 = reader.read_le().await?;
            let offset: u32 = reader.read_le().await?;
            // #[bw(calc = comment.len() as u16)]
            let comment_length: u16 = reader.read_le().await?;
            // #[br(count = comment_length)]
            let comment: Vec<u8> = reader.read_le_args((comment_length as u64, ())).await?;
            if *model == ZipModel::Parse {
                reader.set_position(offset as u64).await?; // .seek(SeekFrom::Start(offset as u64)).await?;
            }
            read_bytes(reader.position().await? - pos).await?;
            let directories: IndexDirectory<T> = reader
                .read_le_args((model, config, entries, read_bytes))
                .await?;
            Ok(Self {
                config: config.clone(),
                crc32_computer,
                eocd_offset,
                magic,
                number_of_disk,
                directory_starts,
                number_of_directory_disk,
                entries,
                size,
                offset,
                comment_length,
                comment,
                directories,
            })
        }
    }
}
pub struct IndexDirectory<T>(pub IndexMap<String, Directory<T>>)
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config;

impl<T> DerefMut for IndexDirectory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl<T> Deref for IndexDirectory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    type Target = IndexMap<String, Directory<T>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T> BinRead for IndexDirectory<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    type Args<'a>
        = (
        &'a ZipModel,
        &'a T::Config,
        u16,
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
        Self: 'a,
    {
        async move {
            let (model, config, count, read_bytes) = args;
            let mut seen = HashSet::new();
            let mut directories = IndexMap::with_capacity(count as usize);
            for index in 0..count {
                let dir: Directory<T> =
                    Directory::read_options(reader, endian, (index, model, config, read_bytes))
                        .await?;
                let name = String::from_utf8(dir.file_name.inner.clone())
                    .map_err(|e| Error::Err(Box::new(e)))?;
                let lower = name.to_lowercase();
                if seen.insert(lower) {
                    directories.insert(name, dir);
                }
            }
            Ok(IndexDirectory(directories))
        }
    }
}
impl<T> BinWrite for IndexDirectory<T>
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
            let (model,) = args;
            for (_, v) in &self.0 {
                v.write_options(writer, endian, (model,)).await?;
            }
            Ok(())
        }
    }
}
impl<T> FastZip<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    pub fn enable_crc32_computer(&mut self) {
        self.crc32_computer = true.into();
    }
    pub fn disable_crc32_computer(&mut self) {
        self.crc32_computer = false.into();
    }
}
impl<T> FastZip<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    pub fn empty() -> FastZip<T> {
        Self {
            config: Default::default(),
            crc32_computer: Default::default(),
            eocd_offset: 0,
            magic: Default::default(),
            number_of_disk: 0,
            directory_starts: 0,
            number_of_directory_disk: 0,
            entries: 0,
            size: 0,
            offset: 0,
            comment_length: 0,
            comment: vec![],
            directories: IndexDirectory(IndexMap::new()),
        }
    }
    pub fn parse(reader: &mut T) -> impl Future<Output = BinResult<FastZip<T>>> + Send {
        async move {
            let config = reader.config().clone();
            let mut reader = BufReader::with_capacity(32 * 1024, reader);
            let zip = FastZip::read_le_args(
                &mut reader,
                (&ZipModel::Parse, &config, &mut |_bytes| {
                    Box::pin(async { Ok(()) })
                }),
            )
            .await?;
            reader.rewind_position().await?;
            Ok(zip)
        }
    }
    pub fn parse_with_callback(
        reader: &mut T,
        callback: impl FnMut(u64, u64) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> + Send,
    ) -> impl Future<Output = BinResult<FastZip<T>>> + Send {
        async move {
            let config = reader.config().clone();
            let pos = reader.position().await?;
            let total = reader.seek_end().await?;
            reader.set_position(pos).await?;
            let mut sum = 0;
            let mut buffered = 0;
            let mut callback = Self::create_adapter(total, &mut buffered, &mut sum, callback);
            let mut reader = BufReader::with_capacity(32 * 1024, reader);
            let result =
                FastZip::read_le_args(&mut reader, (&ZipModel::Parse, &config, &mut callback))
                    .await;
            reader.rewind_position().await?;
            callback(0).await?;
            result
        }
    }
    pub fn remove_file(&mut self, file_name: &str) {
        self.directories.swap_remove(file_name);
    }
    pub fn save_file(
        &mut self,
        mut data: T,
        file_name: &str,
    ) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            data.seek_start().await?;
            if let Some(dir) = self.directories.get_mut(file_name) {
                return dir.put_data(data).await;
            }
            self.add_file(data, file_name).await?;
            Ok(())
        }
    }
    pub fn add_directory(&mut self, mut dir: Directory<T>) -> BinResult<()> {
        if dir.file_name.inner != dir.file.file_name.inner {
            dir.file.file_name = dir.file_name.clone();
        }
        let name =
            String::from_utf8(dir.file_name.inner.clone()).map_err(|e| Error::Err(Box::new(e)))?;
        self.directories.insert(name, dir);
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
    pub fn create_dir(
        mut data: T,
        file_name: &str,
    ) -> impl Future<Output = BinResult<Directory<T>>> + Send {
        async move {
            data.seek_start().await?;
            let length = data.length().await?;
            let uncompressed_size = length as u32;
            let crc_32_uncompressed_data = 0; //data.crc32_value();

            let mut buffer = vec![0u8; std::cmp::min(uncompressed_size as usize, 1024)];
            data.read_exact(&mut buffer).await?;
            data.seek_start().await?;
            let internal_file_attributes = if Self::is_binary(&buffer) { 0 } else { 1 };

            let file_name = Name {
                inner: file_name.as_bytes().to_vec(),
            };
            let extra_fields: ExtraList = vec![].into();
            //     vec![
            //     Extra::UnixExtendedTimestamp {
            //         mtime: Some(1736154637),
            //         atime: Some(1736195293),
            //         ctime: None,
            //     },
            //     Extra::UnixAttrs { uid: 503, gid: 20 },
            // ]
            // .into();
            // let mut ext_bytes = Cursor::new(vec![]);
            // ext_bytes.write_le(&extra_fields)?;
            // let extra_field_length = ext_bytes.get_ref().len() as u16;
            let mut directory = Directory {
                // _config: PhantomData,
                compressed: false,
                data: Some(data),
                created_zip_spec: 0x1E, //3.0
                created_os: 0x03,       //Uninx
                extract_zip_spec: 0x0E, //2.0
                extract_os: 0,          //MS-DOS
                flags: 0,
                compression_method: CompressionMethod::Deflate,
                last_modification_time: 39620,
                last_modification_date: 23170,
                crc_32_uncompressed_data,
                compressed_size: 0,
                uncompressed_size,
                // extra_field_length,
                number_of_starts: 0,
                internal_file_attributes,
                // external_file_attributes: 2175008768,
                offset_of_local_file_header: 0,
                file_name: file_name.clone(),
                extra_fields: extra_fields.clone(),
                file_comment: vec![],
                file: ZipFile {
                    extract_zip_spec: 0,
                    flags: 0,
                    extract_os: 0, //MS-DOS
                    compression_method: CompressionMethod::Deflate,
                    last_modification_time: 39620,
                    last_modification_date: 23170,
                    crc_32_uncompressed_data,
                    compressed_size: 0,
                    uncompressed_size,
                    // extra_field_length,
                    file_name_length: 0,
                    extra_field_length: 0,
                    file_name: file_name.clone(),
                    extra_fields,
                    data_descriptor: None,
                    data_position: 0,
                },
                sha_value: None,
            };
            let dir = directory.is_dir();
            if dir {
                directory.flags = 0;
                directory.file.flags = 0;
                directory.compression_method = CompressionMethod::Store;
                directory.file.compression_method = CompressionMethod::Store;
            }
            Ok(directory)
        }
    }
    pub fn add_file(
        &mut self,
        data: T,
        file_name: &str,
    ) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            let dir = Self::create_dir(data, file_name).await?;
            let lower = file_name.to_lowercase();
            let mut seen = IndexMap::new();
            for (name, _) in &self.directories.0 {
                seen.insert(name.to_lowercase(), name.to_string());
            }
            if let Some(full_name) = seen.get(&lower) {
                self.directories.swap_remove(full_name);
            }
            self.directories.insert(file_name.to_string(), dir);
            Ok(())
        }
    }
    pub fn create_adapter<'a, CB>(
        total: u64,
        buffered: &'a mut u64,
        sum: &'a mut u64,
        mut cb: CB,
    ) -> impl FnMut(u64) -> Pin<Box<dyn Future<Output = BinResult<()>> + Send>> + Send + 'a
    where
        CB: FnMut(u64, u64) -> Pin<Box<dyn Future<Output = BinResult<()>> + Send>> + Send + 'a,
    {
        move |bytes| {
            if bytes == 0 {
                if *buffered == 0 {
                    return Box::pin(async { Ok(()) });
                }
                let result = cb(total, *sum);
                *buffered = 0;
                return result;
            }
            *buffered += bytes;
            *sum += bytes;
            if *buffered >= 1024 * 1024 {
                *buffered = 0;
                cb(total, *sum)
            } else {
                Box::pin(async { Ok(()) })
            }
        }
    }
    pub fn computer_un_compress_size(&mut self) -> impl Future<Output = BinResult<u64>> + Send {
        async move {
            let mut total_size = 0;
            for (_, director) in &mut self.directories.0 {
                total_size += if !director.compressed
                    && director.compression_method == CompressionMethod::Deflate
                {
                    if let Some(data) = &mut director.data {
                        data.length().await?
                    } else {
                        0
                    }
                } else {
                    0
                }
            }
            Ok(total_size)
        }
    }

    pub(crate) fn write_eocd<R: Write + Seek + Send>(
        &mut self,
        writer: &mut R,
    ) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            writer.write_le(&self.magic).await?;
            writer.write_le(&self.number_of_disk).await?;
            writer.write_le(&self.directory_starts).await?;
            writer.write_le(&self.number_of_directory_disk).await?;
            writer.write_le(&(self.directories.len() as u16)).await?;
            writer.write_le(&self.size).await?;
            writer.write_le(&self.offset).await?;
            writer.write_le(&(self.comment.len() as u16)).await?;
            writer.write_all(&self.comment).await?;
            Ok(())
        }
    }
}

// #[binrw::parser(reader, endian)]
pub fn parse_eocd_offset<R: Read + Seek + Send>(
    reader: &mut R,
    endian: Endian,
    model: &ZipModel,
) -> impl Future<Output = BinResult<u64>> + Send {
    async move {
        if *model == ZipModel::Bin {
            return Ok(0);
        }
        let max_eocd_size: u64 = u16::MAX as u64 + 22;
        let mut search_size: u64 = 22; //最快的搜索
        let file_size = reader.length().await?;
        let pos = reader.position().await?;

        if file_size < search_size {
            return Err(Error::BadMagic(
                pos,
                "file size le search size, not a zip file".to_string(),
            ));
        }
        // let eocd_magic: u32 = Magic::EoCd.into();
        loop {
            // 确保搜索范围不超过 EOCD 的最大大小
            search_size = search_size.min(max_eocd_size);
            reader.seek(SeekFrom::End(-(search_size as i64))).await?;
            for i in 0..search_size - 3 {
                let pos = reader.position().await?;
                // stream.pin()?;
                // let magic: u32 = stream.read_value()?;
                let magic: u32 = reader.read_type(endian).await?;
                reader.set_position(pos + 1).await?;
                // reader.seek(SeekFrom::Start(pos)).await?;
                // reader.seek(SeekFrom::Current(1)).await?;
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
        Err(Error::BadMagic(pos, "not a zip file".to_string()))
    }
}

pub fn is_dir(file_name: &[u8]) -> bool {
    matches!(file_name.last(), Some(b'/') | Some(b'\\'))
}
