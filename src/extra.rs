use binrw::io::read::Read;
use binrw::io::seek::Seek;
use binrw::io::write::Write;
use binrw::{BinRead, BinReaderExt, BinResult, BinWrite, BinWriterExt, Endian, Error};
use std::io::Cursor;

#[derive(Clone)]
pub enum Extra {
    NTFS {
        mtime: u64,
        atime: u64,
        ctime: u64,
    },
    UnixOldExtendedTimestamp {
        version: u16,
        mode: u16,
        mtime: u32,
        atime: Option<u32>,
    },
    UnixExtendedTimestamp {
        mtime: Option<u32>,
        atime: Option<u32>,
        ctime: Option<u32>,
    },
    UnixAttrs {
        uid: u32,
        gid: u32,
    },
}
// pub enum ExtraType {
//     NTFS = 0x5855,
//     UnixExtendedTimestamp = 0x5858,
//     UnixAttrs = 0x5859,
// }

impl BinWrite for Extra {
    type Args<'a> = ();

    fn write_options<'a, 'w, W>(
        &'a self,
        writer: &'w mut W,
        endian: Endian,
        _args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<()>> + Send + 'w
    where
        'a: 'w,
        W: Write + Seek + Send,
        Self: Sync + 'a,
    {
        async move {
            let mut output = Cursor::new(Vec::new());
            let header_id: u16 = match self {
                Extra::NTFS {
                    mtime,
                    atime,
                    ctime,
                    ..
                } => {
                    output.write_type(&0_u32, endian).await?;
                    output.write_type(&1_u16, endian).await?; //Tag1
                    output.write_type(&24_u16, endian).await?; //Size1
                    output.write_type(mtime, endian).await?;
                    output.write_type(atime, endian).await?;
                    output.write_type(ctime, endian).await?;
                    0x000a
                }
                Extra::UnixExtendedTimestamp {
                    mtime,
                    atime,
                    ctime,
                    ..
                } => {
                    let mut flags: u8 = 0;
                    let mut times = Vec::new();
                    if let Some(mtime) = mtime {
                        flags |= 1;
                        times.push(mtime);
                    }
                    if let Some(atime) = atime {
                        flags |= 2;
                        times.push(atime);
                    }
                    if let Some(ctime) = ctime {
                        flags |= 4;
                        times.push(ctime);
                    }
                    output.write_type(&flags, endian).await?;
                    for time in times {
                        output.write_type(&time, endian).await?;
                    }
                    0x5455
                }
                Extra::UnixAttrs { uid, gid, .. } => {
                    output.write_type(&1_u8, endian).await?;
                    output.write_type(&4_u8, endian).await?;
                    output.write_type(uid, endian).await?;
                    output.write_type(&4_u8, endian).await?;
                    output.write_type(gid, endian).await?;
                    0x7875
                }
                Extra::UnixOldExtendedTimestamp {
                    version,
                    mode,
                    mtime,
                    atime,
                } => {
                    output.write_type(version, endian).await?;
                    output.write_type(mode, endian).await?;
                    output.write_type(mtime, endian).await?;
                    if let Some(atime) = atime {
                        output.write_type(atime, endian).await?;
                    }
                    0x5855
                }
            };
            writer.write_type(&header_id, endian).await?;
            let size = output.get_ref().len() as u16;
            writer.write_type(&size, endian).await?;
            writer.write_all(output.get_ref()).await?;
            Ok(())
        }
    }
}
impl BinRead for Extra {
    type Args<'a> = ();

    fn read_options<'a, 'r, R>(
        reader: &'r mut R,
        endian: Endian,
        _args: Self::Args<'a>,
    ) -> impl Future<Output = BinResult<Self>> + Send + 'r
    where
        'a: 'r,
        R: Read + Seek + Send,
        Self: Send + 'a,
    {
        async move {
            let id: u16 = reader.read_type(endian).await?;
            Ok(match id {
                0x5855 => {
                    let length: u16 = reader.read_type(endian).await?;
                    let mut bytes = vec![0u8; length as usize];
                    reader.read_exact(&mut bytes).await?;
                    let mut data = Cursor::new(bytes);
                    let version = data.read_type(endian).await?;
                    let mode = data.read_type(endian).await?;
                    let mtime = reader.read_type(endian).await?;
                    let atime = if length >= 12 {
                        Some(reader.read_type(endian).await?)
                    } else {
                        None
                    };
                    Self::UnixOldExtendedTimestamp {
                        version,
                        mode,
                        mtime,
                        atime,
                    }
                }
                0x5455 => {
                    let length: u16 = reader.read_type(endian).await?;
                    let mut bytes = vec![0u8; length as usize];
                    reader.read_exact(&mut bytes).await?;
                    let mut data = Cursor::new(bytes);
                    let flags: u8 = data.read_type(endian).await?;
                    let mtime = if flags & 0x01 != 0 {
                        if length >= 5 {
                            Some(data.read_type(endian).await?)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let atime = if flags & 0x02 != 0 {
                        if length >= 9 {
                            Some(data.read_type(endian).await?)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let ctime = if flags & 0x04 != 0 {
                        if length >= 13 {
                            Some(data.read_type(endian).await?)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    Self::UnixExtendedTimestamp {
                        mtime,
                        atime,
                        ctime,
                    }
                }
                0x7875 => {
                    let length: u16 = reader.read_type(endian).await?;
                    let mut bytes = vec![0u8; length as usize];
                    reader.read_exact(&mut bytes).await?;
                    let mut data = Cursor::new(bytes);
                    let _version: u8 = data.read_type(endian).await?;
                    let _uid_size: u8 = data.read_type(endian).await?;
                    let uid: u32 = data.read_type(endian).await?;
                    let _gid_size: u8 = data.read_type(endian).await?;
                    Self::UnixAttrs {
                        uid,
                        gid: data.read_type(endian).await?,
                    }
                }
                0x000A => {
                    let length: u16 = reader.read_type(endian).await?;
                    let mut bytes = vec![0u8; length as usize];
                    reader.read_exact(&mut bytes).await?;
                    let mut data = Cursor::new(bytes);
                    let _reserved: u32 = data.read_type(endian).await?;
                    let _tag: u16 = data.read_type(endian).await?;
                    let _size: u16 = data.read_type(endian).await?;
                    let mtime: u64 = data.read_type(endian).await?;
                    let atime: u64 = data.read_type(endian).await?;
                    let ctime: u64 = data.read_type(endian).await?;
                    Self::NTFS {
                        mtime,
                        atime,
                        ctime,
                    }
                }
                _ => {
                    let pos = reader.position().await?;
                    return Err(Error::BadMagic(pos, format!("Extra id {} not match", id)));
                }
            })
        }
    }
}
