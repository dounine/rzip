use crate::directory::{Bool, CompressionMethod, Directory, Name};
use crate::file::{ExtraList, ZipFile};
use crate::util::stream_length;
use alloc::alloc;
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, Error, binrw};
use indexmap::IndexMap;
use miniz_oxide::deflate::CompressionLevel;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::{Deref, DerefMut};

#[binrw]
#[brw(repr(u8))]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ZipModel {
    Parse,
    Package,
    Bin,
}

#[binrw]
#[brw(repr(u32))]
#[derive(Debug, Clone, PartialEq)]
pub enum Magic {
    EoCd = 0x06054b50,
    Directory = 0x02014b50,
    File = 0x04034b50,
}
impl Default for Magic {
    fn default() -> Self {
        Self::EoCd
    }
}

#[binrw]
#[brw(little, magic = 0x04034b50_u32, import(model:ZipModel))]
#[derive(Debug, Clone)]
pub struct FastZip<T: Read + Write + Seek + Clone + Default> {
    #[brw(if(model == ZipModel::Bin))]
    crc32_computer: Bool,
    #[br(parse_with = parse_eocd_offset,args(model.clone(),))]
    #[bw(ignore)]
    pub eocd_offset: u64,
    #[br(if(model==ZipModel::Parse),seek_before = SeekFrom::End(-(eocd_offset as i64)))]
    #[bw(if(model==ZipModel::Parse))]
    magic: Magic,
    pub number_of_disk: u16,
    pub directory_starts: u16,
    pub number_of_directory_disk: u16,
    #[bw(calc = directories.len() as u16)]
    pub entries: u16,
    pub size: u32,
    pub offset: u32,
    #[bw(calc = comment.len() as u16)]
    pub comment_length: u16,
    #[br(count = comment_length)]
    pub comment: Vec<u8>,
    #[br(seek_before = if model == ZipModel::Parse {
            SeekFrom::Start(offset as u64)
        } else {
            SeekFrom::Current(0)
        },args(&model, entries,)
    )]
    #[bw(if(model == ZipModel::Bin),args(&model,))]
    pub directories: IndexDirectory<T>,
}
#[derive(Debug, Clone)]
pub struct IndexDirectory<T>(pub IndexMap<String, Directory<T>>)
where
    T: Read + Write + Seek + Clone + Default;

impl<T> DerefMut for IndexDirectory<T>
where
    T: Read + Write + Seek + Clone + Default,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl<T> Deref for IndexDirectory<T>
where
    T: Read + Write + Seek + Clone + Default,
{
    type Target = IndexMap<String, Directory<T>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T> BinRead for IndexDirectory<T>
where
    T: Read + Write + Seek + Clone + Default,
{
    type Args<'a> = (&'a ZipModel, u16);

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let (model, size) = args;
        let mut directories = IndexMap::new();
        for _ in 0..size {
            let dir: Directory<T> = reader.read_type_args(endian, (model.clone(),))?;
            let name =
                String::from_utf8(dir.file_name.inner.clone()).map_err(|e| Error::Custom {
                    pos: 0,
                    err: Box::new(e),
                })?;
            directories.insert(name, dir);
        }
        Ok(IndexDirectory(directories))
    }
}
impl<T> BinWrite for IndexDirectory<T>
where
    T: Read + Write + Seek + Clone + Default,
{
    type Args<'a> = (&'a ZipModel,);

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        args: Self::Args<'_>,
    ) -> BinResult<()> {
        let (model,) = args;
        for (_, v) in &self.0 {
            v.write_options(writer, endian, (model.clone(),))?;
        }
        Ok(())
    }
}
impl<T> FastZip<T>
where
    T: Read + Write + Seek + Clone + Default,
{
    pub fn enable_crc32_computer(&mut self) {
        self.crc32_computer = true.into();
    }
    pub fn disable_crc32_computer(&mut self) {
        self.crc32_computer = false.into();
    }
}
impl<D> FastZip<D>
where
    D: Read + Write + Seek + Clone + Default,
{
    pub fn parse<T: Read + Seek>(reader: &mut T) -> BinResult<FastZip<D>> {
        FastZip::read_le_args(reader, (ZipModel::Parse,))
    }
    pub fn remove_file(&mut self, file_name: &str) {
        self.directories.swap_remove(file_name);
    }
    pub fn save_file(&mut self, mut data: D, file_name: &str) -> BinResult<()> {
        data.seek(SeekFrom::Start(0))?;
        if let Some(dir) = self.directories.get_mut(file_name) {
            return dir.put_data(data);
        }
        self.add_file(data, file_name)?;
        Ok(())
    }
    pub fn add_directory(&mut self, mut dir: Directory<D>) -> BinResult<()> {
        if dir.file_name.inner != dir.file.file_name.inner {
            dir.file.file_name = dir.file_name.clone();
        }
        let name = String::from_utf8(dir.file_name.inner.clone()).map_err(|e| Error::Custom {
            pos: 0,
            err: Box::new(e),
        })?;
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
    pub fn add_file(&mut self, mut data: D, file_name: &str) -> BinResult<()> {
        let length = stream_length(&mut data)?;
        let uncompressed_size = length as u32;
        let crc_32_uncompressed_data = 0; //data.crc32_value();
        let compressed_size = uncompressed_size; //data.compress(CompressionLevel::DefaultLevel)? as u32;

        let mut buffer = vec![0u8; std::cmp::min(compressed_size as usize, 1024)];
        data.read_exact(&mut buffer)?;
        data.seek(SeekFrom::Start(0))?;
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
        let directory = Directory {
            compressed: false.into(),
            data,
            created_zip_spec: 0x1E, //3.0
            created_os: 0x03,       //Uninx
            extract_zip_spec: 0x0E, //2.0
            extract_os: 0,          //MS-DOS
            compression_method: CompressionMethod::Deflate,
            last_modification_time: 39620,
            last_modification_date: 23170,
            crc_32_uncompressed_data,
            compressed_size,
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
                extract_os: 0, //MS-DOS
                compression_method: CompressionMethod::Deflate,
                last_modification_time: 39620,
                last_modification_date: 23170,
                crc_32_uncompressed_data,
                compressed_size,
                uncompressed_size,
                // extra_field_length,
                file_name: file_name.clone(),
                extra_fields,
                data_position: 0,
            },
        };
        let name =
            String::from_utf8(directory.file_name.inner.clone()).map_err(|e| Error::Custom {
                pos: 0,
                err: Box::new(e),
            })?;
        self.directories.insert(name, directory);
        Ok(())
    }
    fn create_adapter<F: FnMut(usize, usize, String)>(
        total: usize,
        sum: &mut usize,
        mut f: F,
    ) -> impl FnMut(usize) {
        move |x| {
            *sum += x;
            f(
                total,
                *sum,
                format!("{:.2}%", (*sum as f64 / total as f64) * 100.0),
            )
        }
    }
    fn computer_un_compress_size(&mut self) -> BinResult<usize> {
        let mut total_size = 0;
        for (_, director) in &mut self.directories.0 {
            total_size += if !director.compressed.value
                && director.compression_method == CompressionMethod::Deflate
            {
                stream_length(&mut director.data)?
            } else {
                0
            }
        }
        Ok(total_size as usize)
    }
    pub fn to_bin<T: Write + Seek>(&self, writer: &mut T) -> BinResult<()> {
        self.write_le_args(writer, (ZipModel::Bin,))?;
        Ok(())
    }
    pub fn from_bin<T: Read + Seek>(reader: &mut T) -> BinResult<Self> {
        reader.seek(SeekFrom::Start(0))?;
        reader.read_type_args(Endian::Little, (ZipModel::Bin,))
    }
    pub fn package<W: Write + Seek>(
        &mut self,
        writer: &mut W,
        compression_level: CompressionLevel,
    ) -> BinResult<()> {
        self.package_with_callback(writer, compression_level, &mut |_total, _size, _format| {})
    }
    pub fn package_with_callback<W: Write + Seek>(
        &mut self,
        writer: &mut W,
        compression_level: CompressionLevel,
        callback: &mut impl FnMut(usize, usize, String),
    ) -> BinResult<()> {
        let mut header = D::default();
        let mut files_size = 0;
        let mut directors_size = 0;
        let mut binding = 0;
        let total_size = self.computer_un_compress_size()?;
        let mut callback = Self::create_adapter(total_size, &mut binding, callback);
        let crc32_computer = self.crc32_computer.value;
        for (name, director) in &mut self.directories.0 {
            director.compress_callback(crc32_computer, &compression_level, &mut callback)?;

            director.offset_of_local_file_header = files_size as u32;
            let mut directory_writer = D::default();
            directory_writer.write_le_args(director, (ZipModel::Package,))?;
            directory_writer.seek(SeekFrom::Start(0))?;
            directors_size += std::io::copy(&mut directory_writer, &mut header)?;

            let mut file_writer = D::default();
            let file = &director.file;
            file_writer.write_le_args(
                &file,
                (
                    ZipModel::Package,
                    director.compressed_size,
                    director.uncompressed_size,
                    director.crc_32_uncompressed_data,
                ),
            )?;
            file_writer.seek(SeekFrom::Start(0))?;
            let file_writer_length = std::io::copy(&mut file_writer, writer)?;

            let file_data_length = if !director.file_name.inner.ends_with(&[b'/']) {
                let mut data = &mut director.data;
                data.seek(SeekFrom::Start(0))?;
                std::io::copy(&mut data, writer)?
            } else {
                0
            };
            files_size += file_writer_length + file_data_length;
        }
        header.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut header, writer)?;
        self.size = directors_size as u32;
        // self.entries = self.directories.len() as u16;
        self.number_of_directory_disk = self.directories.len() as u16;
        self.offset = files_size as u32;
        self.write_eocd(writer)?;
        writer.seek(SeekFrom::Start(0))?;
        Ok(())
    }
    fn write_eocd<T: Write + Seek>(&mut self, writer: &mut T) -> BinResult<()> {
        writer.write_le(&self.magic)?;
        writer.write_le(&self.number_of_disk)?;
        writer.write_le(&self.directory_starts)?;
        writer.write_le(&self.number_of_directory_disk)?;
        writer.write_le(&(self.directories.len() as u16))?;
        writer.write_le(&self.size)?;
        writer.write_le(&self.offset)?;
        writer.write_le(&(self.comment.len() as u16))?;
        writer.write_all(&self.comment)?;
        Ok(())
    }
}

#[binrw::parser(reader, endian)]
pub fn parse_eocd_offset(model: ZipModel) -> BinResult<u64> {
    if model == ZipModel::Bin {
        return Ok(0);
    }
    let max_eocd_size: u64 = u16::MAX as u64 + 22;
    let mut search_size: u64 = 22; //最快的搜索
    let file_size = stream_length(reader)?;
    let pos = reader.stream_position()?;

    if file_size < search_size {
        return Err(Error::BadMagic {
            pos,
            found: Box::new("file size le search size, not a zip file"),
        });
    }
    // let eocd_magic: u32 = Magic::EoCd.into();
    loop {
        // 确保搜索范围不超过 EOCD 的最大大小
        search_size = search_size.min(max_eocd_size);
        reader.seek(SeekFrom::End(-(search_size as i64)))?;
        for i in 0..search_size - 3 {
            let pos = reader.stream_position()?;
            // stream.pin()?;
            // let magic: u32 = stream.read_value()?;
            let magic: u32 = BinRead::read_options(reader, endian, ())?;
            reader.seek(SeekFrom::Start(pos))?;
            reader.seek(SeekFrom::Current(1))?;
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
    Err(Error::BadMagic {
        pos,
        found: Box::new("not a zip file"),
    })
}
