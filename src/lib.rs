extern crate alloc;

pub mod directory;
pub mod extra;
pub mod file;
pub mod util;
pub mod zip;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}
pub use miniz_oxide::deflate::CompressionLevel;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
