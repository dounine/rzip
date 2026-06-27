use binrw::io::{BufWriter, Read, Seek, Write};
use binrw::{BinResult, BinWriterExt, Error};
use indexmap::IndexMap;
use miniz_oxide::deflate::CompressionLevel;
use std::io::SeekFrom;
use std::pin::Pin;

use crate::directory::CompressionMethod;
use crate::file::DataDescriptor;
use crate::zip::{Config, FastZip, StreamDefault, ZipModel};

struct CompressTask {
    file_index: usize,
    pos: u64,
    tx: tokio::sync::mpsc::Sender<FileTask>,
}
impl binrw::io::Seek for CompressTask {
    fn seek(
        &mut self,
        pos: std::io::SeekFrom,
    ) -> impl Future<Output = std::io::Result<u64>> + Send {
        async move {
            let current_pos = self.pos;
            let new_pos = match pos {
                SeekFrom::Start(p) => p,
                SeekFrom::End(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "seek after end",
                    ));
                }
                SeekFrom::Current(p) => {
                    if p < 0 {
                        if let Some(res) = current_pos.checked_sub((-p) as u64) {
                            res
                        } else {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "seek before start",
                            ));
                        }
                    } else {
                        current_pos + p as u64
                    }
                }
            };
            self.pos = new_pos;
            Ok(new_pos)
        }
    }
}
impl binrw::io::Write for CompressTask {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = std::io::Result<usize>> + Send {
        async move {
            self.pos += buf.len() as u64;
            self.tx
                .send(FileTask::CompressData {
                    file_index: self.file_index,
                    buf: buf.to_vec(),
                })
                .await
                .map(|_| buf.len())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        }
    }

    fn flush(&mut self) -> impl Future<Output = std::io::Result<()>> + Send {
        async move {
            self.tx
                .send(FileTask::CompressFlush {
                    file_index: self.file_index,
                })
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        }
    }
}
enum FileTask {
    CompressData { file_index: usize, buf: Vec<u8> },
    CompressFlush { file_index: usize },
    CompressDone { file_index: usize },
    Progress { bytes: u64 },
}

impl<T> FastZip<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    pub fn package(
        &mut self,
        writer: &mut T,
        compression_level: CompressionLevel,
    ) -> impl Future<Output = BinResult<()>> + Send {
        async move {
            self.package_with_callback(writer, compression_level, 1, &mut |_total, _size| {
                Box::pin(async { Ok(()) })
            })
            .await?;
            Ok(())
        }
    }
    pub fn package_with_callback<F>(
        &mut self,
        writer: &mut T,
        compression_level: CompressionLevel,
        large_file_speed: u32,
        callback: &mut F,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        F: FnMut(u64, u64) -> Pin<Box<dyn Future<Output = BinResult<()>> + Send>> + Send,
    {
        #[cfg(feature = "parallel")]
        {
            return self.package_with_callback_parallel(
                writer,
                compression_level,
                large_file_speed,
                callback,
            );
        }
        #[cfg(not(feature = "parallel"))]
        {
            return self.package_with_callback_single(writer, compression_level, callback);
        }
    }
    #[cfg(not(feature = "parallel"))]
    pub(crate) fn package_with_callback_single<F>(
        &mut self,
        writer: &mut T,
        compression_level: CompressionLevel,
        large_file_speed: u32,
        callback: &mut F,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        F: FnMut(u64, u64) -> Pin<Box<dyn Future<Output = BinResult<()>> + Send>> + Send,
    {
        async move {
            let mut files_size = 0;
            let mut directors_size = 0;
            let mut binding = 0;
            let total_un_compress_size = self.computer_un_compress_size().await?;
            let mut buffered = 0;
            let mut callback = Self::create_adapter(
                total_un_compress_size,
                &mut buffered,
                &mut binding,
                callback,
            );
            let crc32_computer = self.crc32_computer;
            let config = writer.config().clone();
            let mut writer = BufWriter::with_capacity(32 * 1024, writer);
            let mut sorted_dirs: Vec<_> = self.directories.0.iter_mut().collect();
            sorted_dirs.sort_by(|(_, a), (_, b)| b.compressed_size.cmp(&a.compressed_size));
            //write LOCAL HEADER
            for (index, (_, director)) in sorted_dirs.into_iter().enumerate() {
                let compression_level = if index < large_file_speed as usize {
                    CompressionLevel::BestSpeed
                } else {
                    compression_level
                };
                director.offset_of_local_file_header = files_size as u32;

                let writer_pos_before = writer.position().await?;
                let is_dir = director.is_dir();
                {
                    let file = &mut director.file;
                    if !is_dir && director.compression_method == CompressionMethod::Deflate {
                        director.flags = 0x08;
                        file.flags = 0x08;
                    }
                    writer
                        .write_le_args(file, (&ZipModel::Parse, director.uncompressed_size))
                        .await?;
                }

                if !is_dir {
                    if let Some((crc32, compressed_size)) = director
                        .compress_to_writer_callback(
                            &config,
                            crc32_computer,
                            compression_level,
                            &mut writer,
                            &mut callback,
                        )
                        .await?
                    {
                        director.compressed_size = compressed_size as u32;
                        director.crc_32_uncompressed_data = crc32;
                        director.file.data_descriptor = Some(DataDescriptor {
                            crc32,
                            compressed_size,
                            uncompressed_size: director.uncompressed_size,
                        });
                    } else if let Some(data) = &mut director.data {
                        data.seek_start().await?;
                        binrw::io::copy(data, &mut writer).await?;
                        if director.compression_method == CompressionMethod::Deflate {
                            director.file.data_descriptor = Some(DataDescriptor {
                                crc32: director.file.crc_32_uncompressed_data,
                                compressed_size: director.file.compressed_size,
                                uncompressed_size: director.file.uncompressed_size,
                            });
                            director.file.crc_32_uncompressed_data = 0;
                            director.file.compressed_size = 0;
                        }
                    }
                }
                if let Some(data_descriptor) = &mut director.file.data_descriptor {
                    writer.write_le(data_descriptor).await?;
                }
                let file_writer_length = writer.position().await? - writer_pos_before; //写入LOCAL HEADER长度
                files_size += file_writer_length;
            }

            // write CENTRAL HEADER
            for (_, director) in &mut self.directories.0 {
                let header_pos_before = writer.position().await?;
                if director.file.data_descriptor.is_some() {
                    director.flags = 0x08;
                }
                writer.write_le_args(director, (&ZipModel::Parse,)).await?;
                let header_pos_after = writer.position().await?;
                directors_size += header_pos_after - header_pos_before;
            }
            callback(0).await?;
            self.size = directors_size as u32;
            self.entries = self.directories.len() as u16;
            self.number_of_directory_disk = self.directories.len() as u16;
            self.offset = files_size as u32;
            self.write_eocd(&mut writer).await?;
            writer.flush().await?;
            writer.seek_start().await?;
            Ok(())
        }
    }
    // #[cfg(feature = "parallel")]
    // pub async fn package_with_callback_parallel<'a, 'c: 'a, F>(
    //     &'a mut self,
    //     writer: &'c mut T,
    //     compression_level: CompressionLevel,
    //     large_file_speed: u32,
    //     callback: &'a mut F,
    // ) -> BinResult<()>
    // where
    //     F: FnMut(u64, u64) -> Pin<Box<dyn Future<Output = BinResult<()>> + Send>> + Send,
    // {
    //     use tokio::sync::mpsc;
    //     let total_un_compress_size = self.computer_un_compress_size().await?;
    //     let cfg = writer.config().clone(); // 使用 Arc 实现真正的共享
    //     let crc32 = self.crc32_computer;
    //     let (tx, mut rx) = mpsc::channel::<u64>(1024);
    //     let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(
    //         std::thread::available_parallelism()
    //             .map(|n| n.get())
    //             .unwrap_or(4),
    //     ));

    //     let (_, results) = unsafe {
    //         async_scoped::TokioScope::scope_and_collect(|scope| {
    //             scope.spawn(async move {
    //                 let mut processed = 0;
    //                 while let Some(bytes) = rx.recv().await {
    //                     processed += bytes;
    //                     callback(total_un_compress_size, processed).await?;
    //                 }
    //                 Ok(())
    //             });

    //             //大文件在前
    //             let mut sorted_dirs: Vec<_> = self.directories.0.values_mut().collect();
    //             sorted_dirs.sort_by(|a, b| b.compressed_size.cmp(&a.compressed_size));

    //             let mut i = 0;
    //             for dir in sorted_dirs {
    //                 let cfg = &cfg;
    //                 let tx = tx.clone();
    //                 let semaphore = semaphore.clone();
    //                 let compression_level = if i < large_file_speed {
    //                     CompressionLevel::BestSpeed
    //                 } else {
    //                     compression_level
    //                 };
    //                 i += 1;
    //                 scope.spawn(async move {
    //                     let _permit = semaphore.acquire().await.ok();
    //                     let mut f = |bytes: u64| {
    //                         let tx = tx.clone();
    //                         Box::pin(async move {
    //                             let _ = tx.send(bytes).await;
    //                             Ok(())
    //                         })
    //                             as Pin<Box<dyn Future<Output = BinResult<()>> + Send>>
    //                     };
    //                     dir.compress_callback(cfg, crc32, compression_level, &mut f)
    //                         .await
    //                 });
    //             }
    //             drop(tx);
    //         })
    //     }
    //     .await;
    //     for res in results {
    //         res.map_err(|e| binrw::Error::Err(Box::new(e)))?
    //             .map_err(|ee| binrw::Error::Err(Box::new(ee)))?;
    //     }
    //     self.package(writer, compression_level).await?;
    //     Ok(())
    // }
    #[cfg(feature = "parallel")]
    pub fn package_with_callback_parallel<F>(
        &mut self,
        writer: &mut T,
        compression_level: CompressionLevel,
        large_file_speed: u32,
        callback: &mut F,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        F: FnMut(u64, u64) -> Pin<Box<dyn Future<Output = BinResult<()>> + Send>> + Send,
    {
        async move {
            use std::sync::Arc;
            use tokio::sync::Semaphore;
            use tokio::sync::mpsc;

            let mut directors_size = 0;
            let mut binding = 0;
            let total_un_compress_size = self.computer_un_compress_size().await?;
            let mut buffered = 0;
            let mut callback = Self::create_adapter(
                total_un_compress_size,
                &mut buffered,
                &mut binding,
                callback,
            );
            let crc32_computer = self.crc32_computer;
            let config = writer.config().clone();
            let mut writer = BufWriter::with_capacity(32 * 1024, writer);

            let (tx, mut rx) = mpsc::channel(3);
            let semaphore = Arc::new(Semaphore::new(1));

            let mut sorted_dirs: Vec<_> = self.directories.0.iter_mut().collect();
            sorted_dirs.sort_by(|(_, a), (_, b)| b.compressed_size.cmp(&a.compressed_size));

            let sorted_dir_paths: Vec<String> = sorted_dirs
                .iter()
                .map(|(name, _)| name.to_string())
                .collect();

            let merge_listener = {
                let config = config.clone();
                let sorted_dir_paths = sorted_dir_paths.clone();
                async move {
                    use std::vec;

                    let mut stack: IndexMap<usize, (Option<T>, u64)> = IndexMap::new();
                    for (file_index, _) in sorted_dir_paths.iter().enumerate() {
                        stack.insert(file_index, (None, 0));
                    }
                    let mut active_index: Option<usize> = None;
                    let mut sended_sort_files = vec![];
                    while let Some(task) = rx.recv().await {
                        match task {
                            FileTask::CompressData { file_index, buf } => {
                                if active_index == Some(file_index) {
                                    let (data, bytes) = &mut stack[file_index];
                                    if let Some(mut dir) = data.take() {
                                        //在处理下一个文件之前，已经有数据写入，先处理完
                                        dir.seek_start().await?;
                                        let len =
                                            binrw::io::copy(&mut dir, &mut writer).await?;
                                        *bytes += len;
                                    }
                                    *bytes += buf.len() as u64;
                                    writer.write_all(&buf).await?;
                                } else {
                                    if active_index.is_none() {
                                        active_index = Some(file_index);
                                        sended_sort_files.push(file_index);
                                        let (_, bytes) = &mut stack[file_index];
                                        *bytes += buf.len() as u64;
                                        writer.write_all(&buf).await?;
                                    } else {
                                        if stack[file_index].0.is_none() {
                                            let data_stream = T::from_config(&config).await?;
                                            stack.insert(file_index, (Some(data_stream), 0));
                                        }
                                        if let (Some(dir), bytes) = &mut stack[file_index] {
                                            *bytes += buf.len() as u64;
                                            dir.write_all(&buf).await?;
                                        }
                                    }
                                }
                            }
                            FileTask::CompressFlush { file_index } => {
                                if active_index == Some(file_index) {
                                    writer.flush().await?;
                                } else {
                                    if active_index.is_none() {
                                        active_index = Some(file_index);
                                        sended_sort_files.push(file_index);
                                    }
                                    if let (Some(dir), _bytes) = &mut stack[file_index] {
                                        binrw::io::write::Write::flush(dir).await?;
                                    }
                                }
                            }
                            FileTask::Progress { bytes } => {
                                callback(bytes).await?;
                            }
                            FileTask::CompressDone { file_index } => {
                                if active_index == Some(file_index) {
                                    active_index = None;
                                }
                            }
                        }
                    }
                    for (file_index, (data, bytes)) in &mut stack {
                        if !sended_sort_files.contains(file_index) {
                            if let Some(mut dir) = data.take() {
                                if active_index != Some(*file_index) {
                                    dir.seek_start().await?;
                                }
                                let len = binrw::io::copy(&mut dir, &mut writer).await?;
                                *bytes += len;
                            }
                            sended_sort_files.push(*file_index);
                        }
                    }

                    Ok::<_, Error>((sended_sort_files, stack, writer, callback))
                }
            };

            let results = unsafe {
                async_scoped::TokioScope::scope_and_collect(|scope| {
                    for (index, (_file_name, director)) in sorted_dirs.into_iter().enumerate() {
                        use tokio::sync::mpsc::Sender;

                        let tx: Sender<FileTask> = tx.clone();
                        let config = config.clone();
                        let semaphore = semaphore.clone();
                        let compression_level = if index < large_file_speed as usize {
                            CompressionLevel::BestSpeed
                        } else {
                            compression_level
                        };
                        let crc32_computer = crc32_computer;

                        scope.spawn(async move {
                            let index = index;
                            let _permit = semaphore.acquire().await.ok();

                            let is_dir = director.is_dir();

                            let mut local_header_writer = std::io::Cursor::new(vec![]);

                            {
                                let file = &mut director.file;
                                if !is_dir
                                    && director.compression_method == CompressionMethod::Deflate
                                {
                                    director.flags = 0x08;
                                    file.flags = 0x08;
                                }
                                local_header_writer
                                    .write_le_args(
                                        file,
                                        (&ZipModel::Parse, director.uncompressed_size),
                                    )
                                    .await?;
                            }
                            tx.send(FileTask::CompressData {
                                file_index: index,
                                buf: local_header_writer.into_inner(),
                            })
                            .await
                            .map_err(|e| Error::Err(Box::new(e)))?;

                            if !is_dir {
                                let mut progress_callback = |bytes: u64| {
                                    let tx = tx.clone();
                                    Box::pin(async move {
                                        let _ = tx.send(FileTask::Progress { bytes }).await;
                                        Ok(())
                                    })
                                        as Pin<Box<dyn Future<Output = BinResult<()>> + Send>>
                                };

                                let mut write_task = CompressTask {
                                    file_index: index,
                                    pos: 0,
                                    tx: tx.clone(),
                                };
                                if let Some((crc32, compressed_size)) = director
                                    .compress_to_writer_callback(
                                        &config,
                                        crc32_computer,
                                        compression_level,
                                        &mut write_task,
                                        &mut progress_callback,
                                    )
                                    .await?
                                {
                                    director.compressed_size = compressed_size as u32;
                                    director.crc_32_uncompressed_data = crc32;
                                    director.file.data_descriptor = Some(DataDescriptor {
                                        crc32,
                                        compressed_size,
                                        uncompressed_size: director.uncompressed_size,
                                    });
                                } else if let Some(data) = &mut director.data {
                                    data.seek_start().await?;
                                    binrw::io::copy(data, &mut write_task).await?;
                                    if director.compression_method == CompressionMethod::Deflate {
                                        director.file.data_descriptor = Some(DataDescriptor {
                                            crc32: director.file.crc_32_uncompressed_data,
                                            compressed_size: director.file.compressed_size,
                                            uncompressed_size: director.file.uncompressed_size,
                                        });
                                        director.file.crc_32_uncompressed_data = 0;
                                        director.file.compressed_size = 0;
                                    }
                                }
                            }

                            if let Some(data_descriptor) = &mut director.file.data_descriptor {
                                let mut dd_writer = std::io::Cursor::new(vec![]);
                                dd_writer.write_le(data_descriptor).await?;
                                tx.send(FileTask::CompressData {
                                    file_index: index,
                                    buf: dd_writer.into_inner(),
                                })
                                .await
                                .map_err(|e| Error::Err(Box::new(e)))?;
                            }

                            tx.send(FileTask::CompressDone { file_index: index })
                                .await
                                .map_err(|e| Error::Err(Box::new(e)))?;

                            Ok::<_, Error>(())
                        });
                    }

                    drop(tx);
                })
            };

            let (tp2, (_, results)) = tokio::join!(merge_listener, results);
            let (sended_sort_files, stack, mut writer, mut callback) = tp2?;

            for res in results {
                res.map_err(|e| Error::Err(Box::new(e)))??;
            }

            // 创建索引到文件名的映射
            let mut index_to_name = std::collections::HashMap::new();
            for (idx, name) in sorted_dir_paths.iter().enumerate() {
                index_to_name.insert(idx, name.clone());
            }

            let mut files_size = 0;
            for index in sended_sort_files.clone() {
                let name = &index_to_name[&index];
                if let Some(d) = self.directories.0.get_mut(name) {
                    d.offset_of_local_file_header = files_size as u32;
                }
                files_size += stack[index].1;
            }
            for index in sended_sort_files {
                let name = &index_to_name[&index];
                let director = self.directories.0.get_mut(name).unwrap();
                let header_pos_before = writer.position().await?;
                if director.file.data_descriptor.is_some() {
                    director.flags = 0x08;
                }
                writer.write_le_args(director, (&ZipModel::Parse,)).await?;
                let header_pos_after = writer.position().await?;
                directors_size += header_pos_after - header_pos_before;
            }

            callback(0).await?;
            self.size = directors_size as u32;
            self.entries = self.directories.len() as u16;
            self.number_of_directory_disk = self.directories.len() as u16;
            self.offset = files_size as u32;
            self.write_eocd(&mut writer).await?;
            writer.flush().await?;

            Ok(())
        }
    }
}
