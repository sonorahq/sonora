//! Collection v2 paging and mutation messages.

#[derive(prost::Message)]
pub struct CollectionItem {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(int32, optional, tag = "2")]
    pub added_at: Option<i32>,
    #[prost(bool, tag = "3")]
    pub is_removed: bool,
    // #[prost(string, optional, tag = "4")]
    // pub context_uri: Option<String>,
}

#[derive(prost::Message)]
pub struct PageRequest {
    #[prost(string, tag = "1")]
    pub username: String,
    #[prost(string, tag = "2")]
    pub set: String,
    #[prost(string, tag = "3")]
    pub token: String,
    #[prost(int32, tag = "4")]
    pub limit: i32,
}

#[derive(prost::Message)]
pub struct PageResponse {
    #[prost(message, repeated, tag = "1")]
    pub items: Vec<CollectionItem>,
    #[prost(string, tag = "2")]
    pub next: String,
    // #[prost(string, tag = "3")]
    // pub sync_token: String,
}

#[derive(prost::Message)]
pub struct WriteRequest {
    #[prost(string, tag = "1")]
    pub username: String,
    #[prost(string, tag = "2")]
    pub set: String,
    #[prost(message, repeated, tag = "3")]
    pub items: Vec<CollectionItem>,
    #[prost(string, tag = "4")]
    pub update_id: String,
}
