use std::path::Path;

use binrw::{
    BinResult,
    io::{
        Read, Seek, Write,
        bytes::{BytesCallback, BytesToTotalAdapter, TotalBytesCallback},
    },
};

use crate::zip::{Config, FastZip, StreamDefault};

impl<T> FastZip<T>
where
    T: Read + Write + Seek + Send + StreamDefault,
    T::Config: Config,
{
    /// 解压到output，目录不存在则创建
    pub fn unzip<'a, F>(
        &mut self,
        output: &'a Path,
        callback: &'a mut F,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        F: TotalBytesCallback + Send,
    {
        async move {
            if !output.exists() {
                std::fs::create_dir_all(output)?;
            }
            let mut total_bytes = 0;
            for (_, dir) in &mut self.directories.0 {
                if dir.compressed
                    && let Some(data) = &mut dir.data
                {
                    total_bytes += data.length().await?;
                }
            }

            #[cfg(feature = "parallel")]
            {
                use tokio::sync::mpsc;
                let (tx, mut rx) = mpsc::channel::<u64>(1024);
                let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(
                    std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(4),
                ));
                let (_, results) = unsafe {
                    async_scoped::TokioScope::scope_and_collect(|scope| {
                        scope.spawn(async move {
                            let mut processed = 0;
                            while let Some(bytes) = rx.recv().await {
                                processed += bytes;
                                callback.call(processed, total_bytes).await?;
                            }
                            Ok(())
                        });
                        for (file_name, dir) in &mut self.directories.0 {
                            let tx = tx.clone();
                            let semaphore = semaphore.clone();
                            scope.spawn(async move {
                                let file_path = output.join(file_name);
                                if dir.is_dir() {
                                    use binrw::Error;
                                    tokio::fs::create_dir_all(&file_path)
                                        .await
                                        .map_err(|e| Error::Io(e))
                                } else {
                                    let _permit = semaphore.acquire().await.ok();
                                    if let Some(_data) = &mut dir.data {
                                        // 确保文件的父目录存在

                                        use std::{fs::OpenOptions, pin::Pin};

                                        use binrw::io::{
                                            BufWriter, bytes::BytesCallbackFn, cb::WriteCallback,
                                        };
                                        if let Some(parent_dir) = file_path.parent() {
                                            if !parent_dir.exists() {
                                                tokio::fs::create_dir_all(parent_dir).await?;
                                            }
                                        }
                                        let file = OpenOptions::new()
                                            .read(true)
                                            .write(true)
                                            .create(true)
                                            .truncate(true)
                                            .open(file_path)?;
                                        let callback = BytesCallbackFn::new(
                                            |bytes| -> Pin<
                                                Box<
                                                    dyn std::future::Future<Output = BinResult<()>>
                                                        + Send,
                                                >,
                                            > {
                                                let tx = tx.clone();
                                                Box::pin(async move {
                                                    let _ = tx.send(bytes).await;
                                                    Ok(())
                                                })
                                            },
                                        );
                                        // file.set_len(data.length().await?)?;
                                        // let mut mmap = unsafe {
                                        //     use memmap2::MmapMut;
                                        //     MmapMut::map_mut(&file)?
                                        // };
                                        // let mut pos = 0;
                                        // let mut buf = [0u8; 1024 * 8];
                                        // loop {
                                        //     let len = data.read(&mut buf).await?;
                                        //     if len == 0 {
                                        //         break;
                                        //     }
                                        //     mmap[pos..pos + len].copy_from_slice(&buf[..len]);
                                        //     pos += len;
                                        // }
                                        // mmap.flush()?;
                                        // binrw::io::copy(reader, writer)
                                        let file = BufWriter::with_capacity(1024 * 1024, file);
                                        let mut output = WriteCallback::new(file, callback);
                                        dir.decompressed_with_writer(&mut output).await?;
                                        output.flush().await?;
                                    }
                                    Ok(())
                                }
                            });
                        }
                        drop(tx);
                    })
                }
                .await;

                for res in results {
                    res.map_err(|e| binrw::Error::Err(Box::new(e)))??;
                }
            }
            #[cfg(not(feature = "parallel"))]
            {
                use binrw::io::bytes::BytesToTotalAdapter;

                let mut callback = BytesToTotalAdapter::new(total_bytes, callback);

                for (file_name, dir) in &mut self.directories.0 {
                    let file_path = output.join(file_name);
                    if dir.is_dir() {
                        std::fs::create_dir_all(&file_path)?;
                    } else {
                        if let Some(_data) = &mut dir.data {
                            // 确保文件的父目录存在

                            use std::fs::OpenOptions;

                            use binrw::io::{BufWriter, cb::WriteCallback};
                            if let Some(parent_dir) = file_path.parent() {
                                if !parent_dir.exists() {
                                    std::fs::create_dir_all(parent_dir)?;
                                }
                            }
                            let file = OpenOptions::new()
                                .read(true)
                                .write(true)
                                .create(true)
                                .truncate(true)
                                .open(file_path)?;
                            let file = BufWriter::with_capacity(1024 * 1024, file);
                            let mut output = WriteCallback::new(file, callback);
                            dir.decompressed_with_writer(&mut output).await?;
                            output.flush().await?;
                            (_, callback) = output.into_parts();
                        }
                    }
                }
            }
            Ok(())
        }
    }

    pub fn decompress_all_files<'a, F>(
        &mut self,
        callback: &'a mut F,
    ) -> impl Future<Output = BinResult<()>> + Send
    where
        F: TotalBytesCallback + Send,
    {
        async move {
            let mut total_bytes = 0;
            for (_, dir) in &mut self.directories.0 {
                total_bytes += dir.compressed_size as u64;
            }
            let mut callback = BytesToTotalAdapter::new(total_bytes, callback);

            #[cfg(not(feature = "parallel"))]
            {
                for (_, dir) in &mut self.directories.0 {
                    dir.decompressed_with_callback(&mut callback).await?;
                }
            }
            #[cfg(feature = "parallel")]
            {
                use tokio::sync::mpsc;
                let (tx, mut rx) = mpsc::channel::<u64>(1024);
                let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(
                    std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(4),
                ));
                let ((), results) = unsafe {
                    async_scoped::TokioScope::scope_and_collect(|scope| {
                        scope.spawn(async {
                            while let Some(bytes) = rx.recv().await {
                                callback.call(bytes).await?;
                            }
                            Ok(())
                        });
                        let mut sorted_dirs: Vec<_> = self.directories.0.values_mut().collect();
                        sorted_dirs.sort_by(|a, b| b.compressed_size.cmp(&a.compressed_size));

                        for dir in sorted_dirs {
                            let tx = tx.clone();
                            let semaphore = semaphore.clone();
                            scope.spawn(async move {
                                use std::pin::Pin;

                                use binrw::io::bytes::BytesCallbackFn;

                                let _permit = semaphore.acquire().await.ok();
                                let mut callback = BytesCallbackFn::new(
                                    |bytes| -> Pin<
                                        Box<dyn std::future::Future<Output = BinResult<()>> + Send>,
                                    > {
                                        let tx = tx.clone();
                                        Box::pin(async move {
                                            let _ = tx.send(bytes).await;
                                            Ok(())
                                        })
                                    },
                                );
                                dir.decompressed_with_callback(&mut callback).await
                            });
                        }
                        drop(tx);
                    })
                }
                .await;
                for res in results {
                    res.map_err(|e| binrw::Error::Err(Box::new(e)))??;
                }
            }

            Ok(())
        }
    }
    pub async fn decompress_with_files<'a, F>(
        &'a mut self,
        callback: &'a mut F,
        files: &'a [String],
    ) -> BinResult<()>
    where
        F: BytesCallback + Send,
    {
        #[cfg(feature = "parallel")]
        self.decompress_files_parallel(callback, files).await?;
        #[cfg(not(feature = "parallel"))]
        for (file_name, dir) in &mut self.directories.0 {
            if let Some(data) = &mut dir.data {
                data.seek_start().await?;
                if files.contains(file_name) {
                    dir.decompressed_with_callback(callback).await?;
                }
            }
        }
        Ok(())
    }
    #[cfg(feature = "parallel")]
    pub async fn decompress_files_parallel<'a, F>(
        &'a mut self,
        callback: &'a mut F,
        files: &'a [String],
    ) -> BinResult<()>
    where
        F: BytesCallback + Send,
    {
        use std::collections::HashSet;

        use tokio::sync::mpsc;
        let (tx, mut rx) = mpsc::channel::<u64>(1024);
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        ));

        let mut to_decompress = Vec::new();
        let file_set: HashSet<&str> = files.iter().map(|s| s.as_str()).collect();
        for (file_name, dir) in &mut self.directories.0 {
            if file_set.contains(file_name.as_str()) {
                to_decompress.push(dir);
            }
        }

        if to_decompress.is_empty() {
            return Ok(());
        }

        to_decompress.sort_by(|a, b| b.compressed_size.cmp(&a.compressed_size));

        let ((), results) = unsafe {
            async_scoped::TokioScope::scope_and_collect(|scope| {
                scope.spawn(async {
                    while let Some(bytes) = rx.recv().await {
                        callback.call(bytes).await?;
                    }
                    Ok(())
                });

                for dir in to_decompress {
                    let semaphore = semaphore.clone();
                    let tx = tx.clone();
                    scope.spawn(async move {
                        use std::pin::Pin;

                        use binrw::io::bytes::BytesCallbackFn;

                        let _permit = semaphore.acquire().await.ok();
                        let mut callback = BytesCallbackFn::new(
                            |bytes| -> Pin<
                                Box<dyn std::future::Future<Output = BinResult<()>> + Send>,
                            > {
                                let tx = tx.clone();
                                Box::pin(async move {
                                    let _ = tx.send(bytes).await;
                                    Ok(())
                                })
                            },
                        );
                        dir.decompressed_with_callback(&mut callback).await
                    });
                }
                drop(tx);
            })
        }
        .await;

        for res in results {
            res.map_err(|e| binrw::Error::Err(Box::new(e)))??;
        }

        Ok(())
    }
}
