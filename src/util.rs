use binrw::BinResult;
use binrw::io::read::Read;
use binrw::io::seek::Seek;
use std::io::SeekFrom;

pub fn stream_length<R: Read + Seek + Send>(
    reader: &mut R,
) -> impl Future<Output = BinResult<u64>> + Send {
    async move {
        // 保存当前位置
        let current_pos = reader.stream_position().await?;
        // 移动到末尾获取长度
        let length = reader.seek(SeekFrom::End(0)).await?;
        // 恢复原始位置
        reader.seek(SeekFrom::Start(current_pos)).await?;
        Ok(length)
    }
}
