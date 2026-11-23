//! Storage resolver replies for native playback and downloads.

pub(crate) const ENDPOINT: &str = "/storage-resolve/files/audio/interactive";

#[derive(prost::Message)]
pub struct StorageResolveResponse {
    #[prost(int32, tag = "1")]
    pub result: i32,
    #[prost(string, repeated, tag = "2")]
    pub cdnurl: Vec<String>,
    // #[prost(bytes, tag = "4")]
    // pub fileid: Vec<u8>,
}
