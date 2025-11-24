use std::io::{Read, Seek, SeekFrom};
use binrw::BinResult;

pub fn stream_length<R: Read + Seek>(reader: &mut R) -> BinResult<u64> {
    // 保存当前位置
    let current_pos = reader.stream_position()?;
    // 移动到末尾获取长度
    let length = reader.seek(SeekFrom::End(0))?;
    // 恢复原始位置
    reader.seek(SeekFrom::Start(current_pos))?;
    Ok(length)
}
