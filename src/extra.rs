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
    UnixExtendedTimestamp {
        mtime: Option<i32>,
        atime: Option<i32>,
        ctime: Option<i32>,
    },
    UnixAttrs {
        uid: u32,
        gid: u32,
    },
}

impl BinWrite for Extra {
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
                    let flags: u8 = 3;
                    output.write_type(&flags, endian).await?;
                    if let Some(mtime) = mtime {
                        output.write_type(mtime, endian).await?;
                    }
                    if let Some(atime) = atime {
                        output.write_type(atime, endian).await?;
                    }
                    if let Some(ctime) = ctime {
                        output.write_type(ctime, endian).await?;
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
            };
            writer.write_type(&header_id, endian).await?;
            let size = output.get_ref().len() as u16;
            writer.write_type(&size, endian).await?;
            output.set_position(0);
            binrw::io::copy(&mut output, writer).await?;
            Ok(())
        }
    }
}
impl BinRead for Extra {
    type Args<'a> = ();
    fn read_options<R: Read + Seek + Send>(
        reader: &mut R,
        endian: Endian,
        _args: Self::Args<'_>,
    ) -> impl Future<Output = BinResult<Self>> + Send
    where
        Self: Send,
    {
        async move {
            let id: u16 = reader.read_type(endian).await?;
            Ok(match id {
                0x5855 => {
                    let mut _length: u16 = reader.read_type(endian).await?;
                    let mtime = if _length > 0 {
                        _length -= 4;
                        Some(reader.read_type(endian).await?)
                    } else {
                        None
                    };
                    let atime = if _length > 0 {
                        _length -= 4;
                        Some(reader.read_type(endian).await?)
                    } else {
                        None
                    };
                    let ctime = if _length > 0 {
                        _length -= 4;
                        Some(reader.read_type(endian).await?)
                    } else {
                        None
                    };
                    Self::UnixExtendedTimestamp {
                        mtime,
                        atime,
                        ctime,
                    }
                }
                0x5455 => {
                    let mut length: u16 = reader.read_type(endian).await?;
                    length -= 1;
                    let flags: u8 = reader.read_type(endian).await?;
                    let mtime = if flags & 0x01 != 0 {
                        length -= 4;
                        Some(reader.read_type(endian).await?)
                    } else {
                        None
                    };
                    let atime = if flags & 0x02 != 0 {
                        if length == 0 {
                            None
                        } else {
                            length -= 4;
                            Some(reader.read_type(endian).await?)
                        }
                    } else {
                        None
                    };
                    let ctime = if flags & 0x04 != 0 {
                        if length == 0 {
                            None
                        } else {
                            length -= 4;
                            Some(reader.read_type(endian).await?)
                        }
                    } else {
                        None
                    };
                    if length > 0 {
                        u32::read_options(reader, endian, ()).await?;
                    }
                    if flags & 0xF8 != 0 {
                        let pos = reader.position().await?;
                        return Err(Error::BadMagic {
                            pos,
                            found: Box::new("Flags is invalid in ExtendedTimestamp"),
                        });
                    }
                    Self::UnixExtendedTimestamp {
                        mtime,
                        atime,
                        ctime,
                    }
                }
                0x7875 => {
                    let _length: u16 = reader.read_type(endian).await?;
                    let _version: u8 = reader.read_type(endian).await?;
                    let _uid_size: u8 = reader.read_type(endian).await?;
                    let uid: u32 = reader.read_type(endian).await?;
                    let _gid_size: u8 = reader.read_type(endian).await?;
                    Self::UnixAttrs {
                        uid,
                        gid: reader.read_type(endian).await?,
                    }
                }
                0x000A => {
                    let mut _length: u16 = reader.read_type(endian).await?;
                    let _reserved: u32 = reader.read_type(endian).await?;
                    _length -= 4;
                    let tag: u16 = reader.read_type(endian).await?;
                    _length -= 2;
                    if tag != 0x0001 {
                        let pos = reader.position().await?;
                        return Err(Error::BadMagic {
                            pos,
                            found: Box::new("Tag is invalid in NtfsTimestamp"),
                        });
                    }
                    let size: u16 = reader.read_type(endian).await?;
                    _length -= 2;
                    if size != 24 {
                        let pos = reader.position().await?;
                        return Err(Error::BadMagic {
                            pos,
                            found: Box::new("Invalid NTFS Timestamps size"),
                        });
                    }
                    let mtime: u64 = if _length > 0 {
                        _length -= 8;
                        reader.read_type(endian).await?
                    } else {
                        0
                    };
                    let atime: u64 = if _length > 0 {
                        _length -= 8;
                        reader.read_type(endian).await?
                    } else {
                        0
                    };
                    let ctime: u64 = if _length > 0 {
                        _length -= 8;
                        reader.read_type(endian).await?
                    } else {
                        0
                    };
                    Self::NTFS {
                        mtime,
                        atime,
                        ctime,
                    }
                }
                _ => {
                    let pos = reader.position().await?;
                    return Err(Error::BadMagic {
                        pos,
                        found: Box::new(format!("Extra id {} not match", id)),
                    });
                }
            })
        }
    }
}
