//! Playlist snapshots, permissions, picture registration, and revision-bound mutations.

use std::collections::HashMap;

use bytes::Bytes;

/// Identifies the JPEG staged by Spotify's image upload service.
#[derive(prost::Message)]
pub struct RegisterPlaylistImageRequest {
    #[prost(string, tag = "1")]
    pub upload_token: String,
}

/// The picture ID to attach in a playlist attribute update.
#[derive(prost::Message)]
pub struct RegisterPlaylistImageResponse {
    #[prost(bytes = "bytes", tag = "1")]
    pub picture: Bytes,
}

#[derive(prost::Message)]
pub struct Item {
    #[prost(string, required, tag = "1")]
    pub uri: String,
    #[prost(message, optional, tag = "2")]
    pub attributes: Option<ItemAttributes>,
}

#[derive(prost::Message)]
pub struct MetaItem {
    // #[prost(bytes = "bytes", optional, tag = "1")]
    // pub revision: Option<Bytes>,
    #[prost(message, optional, tag = "2")]
    pub attributes: Option<ListAttributes>,
    #[prost(int32, optional, tag = "3")]
    pub length: Option<i32>,
    #[prost(int64, optional, tag = "4")]
    pub timestamp: Option<i64>,
    #[prost(string, optional, tag = "5")]
    pub owner_username: Option<String>,
    // #[prost(bool, optional, tag = "6")]
    // pub abuse_reporting_enabled: Option<bool>,
    #[prost(message, optional, tag = "7")]
    pub capabilities: Option<Capabilities>,
    // #[prost(int32, repeated, packed = "false", tag = "8")]
    // pub geoblock: Vec<i32>,
    #[prost(sint32, optional, tag = "9")]
    pub status_code: Option<i32>,
}

#[derive(prost::Message)]
pub struct ListItems {
    #[prost(int32, required, tag = "1")]
    pub pos: i32,
    #[prost(bool, required, tag = "2")]
    pub truncated: bool,
    #[prost(message, repeated, tag = "3")]
    pub items: Vec<Item>,
    #[prost(message, repeated, tag = "4")]
    pub meta_items: Vec<MetaItem>,
    // #[prost(message, repeated, tag = "5")]
    // pub available_signals: Vec<Signal>,
    // #[prost(string, optional, tag = "6")]
    // pub continuation_token: Option<String>,
    // #[prost(message, repeated, tag = "7")]
    // pub lens_removed_items: Vec<Item>,
}

#[derive(prost::Message)]
pub struct FormatListAttribute {
    #[prost(string, optional, tag = "1")]
    pub key: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub value: Option<String>,
}

#[derive(prost::Message)]
pub struct PictureSize {
    #[prost(string, optional, tag = "1")]
    pub target_name: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub url: Option<String>,
}

#[derive(prost::Message)]
pub struct ListAttributes {
    #[prost(string, optional, tag = "1")]
    pub name: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub description: Option<String>,
    #[prost(bytes = "bytes", optional, tag = "3")]
    pub picture: Option<Bytes>,
    #[prost(bool, optional, tag = "4")]
    pub collaborative: Option<bool>,
    // #[prost(string, optional, tag = "5")]
    // pub pl3_version: Option<String>,
    // #[prost(bool, optional, tag = "6")]
    // pub deleted_by_owner: Option<bool>,
    #[prost(string, optional, tag = "10")]
    pub client_id: Option<String>,
    #[prost(string, optional, tag = "11")]
    pub format: Option<String>,
    #[prost(message, repeated, tag = "12")]
    pub format_attributes: Vec<FormatListAttribute>,
    #[prost(message, repeated, tag = "13")]
    pub picture_size: Vec<PictureSize>,
    // #[prost(bytes, optional, tag = "14")]
    // pub sequence_context_template: Option<Vec<u8>>,
    // #[prost(bytes, optional, tag = "15")]
    // pub ai_curation_reference_id: Option<Vec<u8>>,
    // #[prost(int32, tag = "16")]
    // pub item_id_algorithm: i32,
    // #[prost(bool, optional, tag = "17")]
    // pub recoverable: Option<bool>,
    // #[prost(bool, optional, tag = "18")]
    // pub is_user_created: Option<bool>,
    // #[prost(message, optional, tag = "19")]
    // pub tag_manifest: Option<TagManifest>,
    // #[prost(message, optional, tag = "20")]
    // pub sort_manifest: Option<SortManifest>,
}

#[derive(prost::Message)]
pub struct ItemAttributes {
    #[prost(string, optional, tag = "1")]
    pub added_by: Option<String>,
    #[prost(int64, optional, tag = "2")]
    pub timestamp: Option<i64>,
    // #[prost(int64, optional, tag = "9")]
    // pub seen_at: Option<i64>,
    #[prost(bool, optional, tag = "10")]
    pub public: Option<bool>,
    #[prost(message, repeated, tag = "11")]
    pub format_attributes: Vec<FormatListAttribute>,
    // #[prost(bytes, optional, tag = "12")]
    // pub item_id: Option<Vec<u8>>,
    // #[prost(message, optional, tag = "13")]
    // pub source_lens: Option<Lens>,
    // #[prost(message, repeated, tag = "14")]
    // pub available_signals: Vec<Signal>,
    // #[prost(message, optional, tag = "15")]
    // pub recommendation_info: Option<RecommendationInfo>,
    // #[prost(bytes, optional, tag = "16")]
    // pub sequence_child_template: Option<Vec<u8>>,
    // #[prost(message, optional, tag = "17")]
    // pub save_source: Option<SaveSource>,
    // #[prost(message, optional, tag = "18")]
    // pub tags: Option<Tags>,
    // #[prost(message, optional, tag = "19")]
    // pub sort_values: Option<SortValues>,
}

#[derive(prost::Message)]
pub struct Add {
    #[prost(int32, optional, tag = "1")]
    pub from_index: Option<i32>,
    #[prost(message, repeated, tag = "2")]
    pub items: Vec<Item>,
    #[prost(bool, optional, tag = "4")]
    pub add_last: Option<bool>,
    // #[prost(bool, optional, tag = "5")]
    // pub add_first: Option<bool>,
    // #[prost(message, optional, tag = "6")]
    // pub add_before_item: Option<Item>,
    // #[prost(message, optional, tag = "7")]
    // pub add_after_item: Option<Item>,
}

#[derive(prost::Message)]
pub struct Rem {
    #[prost(int32, optional, tag = "1")]
    pub from_index: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    pub length: Option<i32>,
    #[prost(message, repeated, tag = "3")]
    pub items: Vec<Item>,
    #[prost(bool, optional, tag = "7")]
    pub items_as_key: Option<bool>,
}

#[derive(prost::Message)]
pub struct Mov {
    #[prost(int32, optional, tag = "1")]
    pub from_index: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    pub length: Option<i32>,
    #[prost(int32, optional, tag = "3")]
    pub to_index: Option<i32>,
    #[prost(message, repeated, tag = "4")]
    pub items: Vec<Item>,
    // #[prost(message, optional, tag = "5")]
    // pub add_before_item: Option<Item>,
    // #[prost(message, optional, tag = "6")]
    // pub add_after_item: Option<Item>,
    // #[prost(bool, optional, tag = "7")]
    // pub add_first: Option<bool>,
    #[prost(bool, optional, tag = "8")]
    pub add_last: Option<bool>,
}

#[derive(prost::Message)]
pub struct ItemAttributesPartialState {
    #[prost(message, required, tag = "1")]
    pub values: ItemAttributes,
    #[prost(int32, repeated, packed = "false", tag = "2")]
    pub no_value: Vec<i32>,
}

#[derive(prost::Message)]
pub struct ListAttributesPartialState {
    #[prost(message, required, tag = "1")]
    pub values: ListAttributes,
    #[prost(int32, repeated, packed = "false", tag = "2")]
    pub no_value: Vec<i32>,
}

#[derive(prost::Message)]
pub struct UpdateItemAttributes {
    #[prost(int32, optional, tag = "1")]
    pub index: Option<i32>,
    #[prost(message, required, tag = "2")]
    pub new_attributes: ItemAttributesPartialState,
    // #[prost(message, optional, tag = "3")]
    // pub old_attributes: Option<ItemAttributesPartialState>,
    #[prost(message, optional, tag = "4")]
    pub item: Option<Item>,
}

#[derive(prost::Message)]
pub struct UpdateListAttributes {
    #[prost(message, required, tag = "1")]
    pub new_attributes: ListAttributesPartialState,
    // #[prost(message, optional, tag = "2")]
    // pub old_attributes: Option<ListAttributesPartialState>,
}

#[derive(prost::Message)]
pub struct Op {
    #[prost(enumeration = "Kind", required, tag = "1")]
    pub kind: i32,
    #[prost(message, optional, tag = "2")]
    pub add: Option<Add>,
    #[prost(message, optional, tag = "3")]
    pub rem: Option<Rem>,
    #[prost(message, optional, tag = "4")]
    pub mov: Option<Mov>,
    #[prost(message, optional, tag = "5")]
    pub update_item_attributes: Option<UpdateItemAttributes>,
    #[prost(message, optional, tag = "6")]
    pub update_list_attributes: Option<UpdateListAttributes>,
    // #[prost(message, optional, tag = "7")]
    // pub update_item_uris: Option<UpdateItemUris>,
}

// #[derive(prost::Message)]
// pub struct UpdateItemUris {
//     #[prost(message, repeated, tag = "1")]
//     pub uri_replacements: Vec<UriReplacement>,
// }
//
// #[derive(prost::Message)]
// pub struct UriReplacement {
//     #[prost(int32, optional, tag = "1")]
//     pub index: Option<i32>,
//     #[prost(message, optional, tag = "2")]
//     pub item: Option<Item>,
//     #[prost(string, optional, tag = "3")]
//     pub new_uri: Option<String>,
// }

#[derive(prost::Message)]
pub struct ChangeInfo {
    #[prost(string, optional, tag = "1")]
    pub user: Option<String>,
    #[prost(int64, optional, tag = "2")]
    pub timestamp: Option<i64>,
    // #[prost(bool, optional, tag = "3")]
    // pub admin: Option<bool>,
    #[prost(bool, optional, tag = "4")]
    pub undo: Option<bool>,
    #[prost(bool, optional, tag = "5")]
    pub redo: Option<bool>,
    #[prost(bool, optional, tag = "6")]
    pub merge: Option<bool>,
    #[prost(bool, optional, tag = "7")]
    pub compressed: Option<bool>,
    // #[prost(bool, optional, tag = "8")]
    // pub migration: Option<bool>,
    // #[prost(int32, optional, tag = "9")]
    // pub split_id: Option<i32>,
    #[prost(message, optional, tag = "10")]
    pub source: Option<SourceInfo>,
    // #[prost(int64, optional, tag = "11")]
    // pub server_timestamp: Option<i64>,
    // #[prost(message, optional, tag = "12")]
    // pub creation_info: Option<CreationInfo>,
}

#[derive(prost::Message)]
pub struct SourceInfo {
    #[prost(int32, optional, tag = "1")]
    pub client: Option<i32>,
    #[prost(string, optional, tag = "3")]
    pub app: Option<String>,
    #[prost(string, optional, tag = "4")]
    pub source: Option<String>,
    #[prost(string, optional, tag = "5")]
    pub version: Option<String>,
    // #[prost(string, optional, tag = "6")]
    // pub server_domain: Option<String>,
    // #[prost(message, optional, tag = "7")]
    // pub partner_info: Option<PartnerInfo>,
}

#[derive(prost::Message)]
pub struct Delta {
    // #[prost(bytes, optional, tag = "1")]
    // pub base_version: Option<Vec<u8>>,
    #[prost(message, repeated, tag = "2")]
    pub ops: Vec<Op>,
    #[prost(message, optional, tag = "4")]
    pub info: Option<ChangeInfo>,
}

#[derive(prost::Message)]
pub struct ListChanges {
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub base_revision: Option<Bytes>,
    #[prost(message, repeated, tag = "2")]
    pub deltas: Vec<Delta>,
    #[prost(bool, optional, tag = "3")]
    pub want_resulting_revisions: Option<bool>,
    // #[prost(bool, optional, tag = "4")]
    // pub want_sync_result: Option<bool>,
    // #[prost(int64, repeated, packed = "false", tag = "6")]
    // pub nonces: Vec<i64>,
}

#[derive(prost::Message)]
pub struct SelectedListContent {
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub revision: Option<Bytes>,
    #[prost(int32, optional, tag = "2")]
    pub length: Option<i32>,
    #[prost(message, optional, tag = "3")]
    pub attributes: Option<ListAttributes>,
    #[prost(message, optional, tag = "5")]
    pub contents: Option<ListItems>,
    // #[prost(message, optional, tag = "6")]
    // pub diff: Option<Diff>,
    // #[prost(message, optional, tag = "7")]
    // pub sync_result: Option<Diff>,
    // #[prost(bytes = "bytes", repeated, tag = "8")]
    // pub resulting_revisions: Vec<Bytes>,
    // #[prost(bool, optional, tag = "9")]
    // pub multiple_heads: Option<bool>,
    // #[prost(bool, optional, tag = "10")]
    // pub up_to_date: Option<bool>,
    // #[prost(int64, repeated, packed = "false", tag = "14")]
    // pub nonces: Vec<i64>,
    #[prost(int64, optional, tag = "15")]
    pub timestamp: Option<i64>,
    #[prost(string, optional, tag = "16")]
    pub owner_username: Option<String>,
    // #[prost(bool, optional, tag = "17")]
    // pub abuse_reporting_enabled: Option<bool>,
    #[prost(message, optional, tag = "18")]
    pub capabilities: Option<Capabilities>,
    // #[prost(int32, repeated, packed = "false", tag = "19")]
    // pub geoblock: Vec<i32>,
    // #[prost(bool, optional, tag = "20")]
    // pub changes_require_resync: Option<bool>,
    // #[prost(int64, optional, tag = "21")]
    // pub created_at: Option<i64>,
    // #[prost(message, optional, tag = "22")]
    // pub applied_lenses: Option<AppliedLenses>,
    // #[prost(bool, optional, tag = "23")]
    // pub is_user_created: Option<bool>,
    // #[prost(message, optional, tag = "24")]
    // pub signal_results: Option<SignalResult>,
}

#[derive(prost::Message)]
pub struct CreateListReply {
    #[prost(string, required, tag = "1")]
    pub uri: String,
    // #[prost(bytes = "bytes", optional, tag = "2")]
    // pub revision: Option<Bytes>,
}

#[derive(prost::Message)]
pub struct PlaylistModificationInfo {
    #[prost(string, optional, tag = "1")]
    pub uri: Option<String>,
    #[prost(bytes = "bytes", optional, tag = "2")]
    pub new_revision: Option<Bytes>,
    // #[prost(bytes = "bytes", optional, tag = "3")]
    // pub parent_revision: Option<Bytes>,
    #[prost(message, repeated, tag = "4")]
    pub ops: Vec<Op>,
}

#[derive(Debug, PartialEq, prost::Enumeration)]
pub enum Kind {
    Unknown = 0,
    Add = 2,
    Rem = 3,
    Mov = 4,
    UpdateItemAttributes = 5,
    UpdateListAttributes = 6,
}

#[derive(prost::Message)]
pub struct Capabilities {
    // #[prost(bool, optional, tag = "1")]
    // pub can_view: Option<bool>,
    // #[prost(bool, optional, tag = "2")]
    // pub can_administrate_permissions: Option<bool>,
    // #[prost(int32, repeated, packed = "false", tag = "3")]
    // pub grantable_level: Vec<i32>,
    // #[prost(bool, optional, tag = "4")]
    // pub can_edit_metadata: Option<bool>,
    #[prost(bool, optional, tag = "5")]
    pub can_edit_items: Option<bool>,
    // #[prost(bool, optional, tag = "6")]
    // pub can_cancel_membership: Option<bool>,
    // #[prost(message, optional, tag = "7")]
    // pub grantable_levels: Option<GrantableLevels>,
    // #[prost(message, optional, tag = "8")]
    // pub list_attribute_capabilities: Option<ListAttributeCapabilities>,
    // #[prost(message, repeated, tag = "9")]
    // pub attribute_kind_capabilities: Vec<AttributeKindCapabilities>,
    // #[prost(bool, optional, tag = "10")]
    // pub can_mix_playlist: Option<bool>,
    // #[prost(bool, optional, tag = "11")]
    // pub can_send_messages: Option<bool>,
    // #[prost(message, optional, tag = "12")]
    // pub administrate_permission_kind_capabilities: Option<AdministratePermissionKindCapabilities>,
    // #[prost(bool, optional, tag = "13")]
    // pub can_delete: Option<bool>,
    // #[prost(bool, optional, tag = "14")]
    // pub can_abuse_report: Option<bool>,
    // #[prost(bool, optional, tag = "15")]
    // pub can_publish_to_profile: Option<bool>,
    // #[prost(bytes, optional, tag = "16")]
    // pub unknown_bytes_16: Option<Vec<u8>>,
    // #[prost(bool, optional, tag = "17")]
    // pub can_modify_speed_playlist: Option<bool>,
    // #[prost(uint64, optional, tag = "18")]
    // pub unknown_varint_18: Option<u64>,
    // #[prost(bool, optional, tag = "19")]
    // pub can_edit_prompt: Option<bool>,
    // #[prost(bool, optional, tag = "20")]
    // pub can_add_prompt: Option<bool>,
    // #[prost(bool, optional, tag = "21")]
    // pub can_browse_history: Option<bool>,
    // #[prost(bool, optional, tag = "22")]
    // pub can_revert_history: Option<bool>,
    // #[prost(bool, optional, tag = "23")]
    // pub can_view_prompt: Option<bool>,
}

/// Batch of playlist and root-list reads sent to `GetLists`.
#[derive(prost::Message)]
pub struct GetListsRequest {
    #[prost(message, repeated, tag = "1")]
    pub requests: Vec<GetListRequest>,
}

/// Identifies one list and selects the fields to return.
#[derive(prost::Message)]
pub struct GetListRequest {
    #[prost(message, required, tag = "1")]
    pub identifier: ListIdentifier,
    #[prost(message, required, tag = "2")]
    pub options: RequestOptions,
}

/// Spotify URI of a playlist or a user’s root list.
#[derive(prost::Message)]
pub struct ListIdentifier {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(bytes = "bytes", tag = "2")]
    pub revision: Bytes,
}

/// Options for reading one list, including the response field selection.
#[derive(prost::Message)]
pub struct RequestOptions {
    #[prost(message, required, tag = "4")]
    pub requested_fields: RequestedFields,
    #[prost(message, optional, tag = "6")]
    pub pagination: Option<Pagination>,
}

/// A bounded window into an ordered list.
#[derive(prost::Message)]
pub struct Pagination {
    #[prost(int32, tag = "1")]
    pub length: i32,
    // #[prost(int32, tag = "3")]
    // pub offset: i32,
}

/// Top-level selection of fields in the list response.
#[derive(prost::Message)]
pub struct RequestedFields {
    #[prost(message, required, tag = "2")]
    pub list_response: ListResponseRequestedFields,
}

/// Selects the fields to return from the response’s list entity.
#[derive(prost::Message)]
pub struct ListResponseRequestedFields {
    #[prost(message, required, tag = "1")]
    pub entity: ListEntityRequestedFields,
}

/// Selects list-level, entry, and referenced-list attributes.
#[derive(prost::Message)]
pub struct ListEntityRequestedFields {
    #[prost(message, optional, tag = "1")]
    pub attributes: Option<AttributesRequestedFields>,
    #[prost(message, optional, tag = "2")]
    pub items: Option<ItemRequestedFields>,
    // #[prost(message, optional, tag = "3")]
    // pub meta_items: Option<MetaItemsRequestedFields>,
}

/// Selects the attributes returned for each list entry.
#[derive(prost::Message)]
pub struct ItemRequestedFields {
    #[prost(message, optional, tag = "2")]
    pub attributes: Option<AttributesRequestedFields>,
}

/// Set of attribute IDs to include in a response.
#[derive(prost::Message)]
pub struct AttributesRequestedFields {
    #[prost(enumeration = "AttributeKind", repeated, tag = "1")]
    pub requested_field: Vec<i32>,
}

/// Per-list results in the order they were requested.
#[derive(prost::Message)]
pub struct GetListsResponse {
    // Decode each embedded message separately so malformed entries cannot hide good rows.
    #[prost(bytes = "bytes", repeated, tag = "1")]
    pub responses: Vec<Bytes>,
}

/// One requested list and its revision or error information.
#[derive(prost::Message)]
pub struct GetListResponse {
    #[prost(message, optional, tag = "1")]
    pub entity: Option<ListEntity>,
    #[prost(message, optional, tag = "2")]
    pub info: Option<ResponseInfo>,
}

/// Playlist or root list with entries and metadata for referenced lists.
#[derive(prost::Message)]
pub struct ListEntity {
    #[prost(message, optional, tag = "1")]
    pub attributes: Option<Attributes>,
    #[prost(message, repeated, tag = "2")]
    pub items: Vec<ListItem>,
    // #[prost(map = "string, message", tag = "3")]
    // pub meta_items: HashMap<String, MetaItem>,
}

// #[derive(prost::Message)]
// pub struct MetaItemsRequestedFields {
//     #[prost(message, optional, tag = "1")]
//     pub attributes: Option<AttributesRequestedFields>,
// }
//
// #[derive(PartialEq, prost::Message)]
// pub struct MetaItem {
//     #[prost(message, optional, tag = "1")]
//     pub attributes: Option<Attributes>,
//     #[prost(message, optional, tag = "2")]
//     pub info: Option<ResponseInfo>,
// }

/// One entry in a playlist or root list.
#[derive(prost::Message)]
pub struct ListItem {
    #[prost(message, optional, tag = "2")]
    pub attributes: Option<Attributes>,
}

/// Projected attributes for a list or entry.
#[derive(prost::Message)]
pub struct Attributes {
    #[prost(string, tag = "1")]
    pub uri: String,
    // #[prost(bytes = "bytes", optional, tag = "2")]
    // pub revision: Option<Bytes>,
    #[prost(string, tag = "3")]
    pub name: String,
    // #[prost(string, optional, tag = "4")]
    // pub description: Option<String>,
    // #[prost(message, optional, tag = "5")]
    // pub updated_at: Option<prost_types::Timestamp>,
    // #[prost(string, optional, tag = "6")]
    // pub added_by_username: Option<String>,
    #[prost(string, tag = "7")]
    pub owner_username: String,
    // #[prost(message, optional, tag = "8")]
    // pub created_at: Option<prost_types::Timestamp>,
    // #[prost(message, optional, tag = "9")]
    // pub capabilities: Option<Capabilities>,
    // #[prost(message, optional, tag = "10")]
    // pub geoblock: Option<Geoblock>,
    // #[prost(int32, optional, tag = "11")]
    // pub status_code: Option<i32>,
    // #[prost(bool, optional, tag = "12")]
    // pub abuse_reporting_enabled: Option<bool>,
    // #[prost(int64, optional, tag = "13")]
    // pub approximate_length: Option<i64>,
    // #[prost(message, optional, tag = "14")]
    // pub available_signals: Option<SignalsList>,
    // #[prost(bool, optional, tag = "15")]
    // pub published_to_profile: Option<bool>,
    // #[prost(message, optional, tag = "16")]
    // pub recommendation_info: Option<RecommendationInfo>,
    // #[prost(message, optional, tag = "17")]
    // pub ai_curation: Option<AiCuration>,
    // #[prost(message, optional, tag = "18")]
    // pub added_at: Option<prost_types::Timestamp>,
    // #[prost(bool, optional, tag = "19")]
    // pub deleted_by_owner: Option<bool>,
    #[prost(bytes = "bytes", tag = "20")]
    pub picture_file_id: Bytes,
    #[prost(message, optional, tag = "21")]
    pub picture_sizes: Option<PictureSizes>,
    // #[prost(string, optional, tag = "22")]
    // pub client_id: Option<String>,
    // #[prost(string, optional, tag = "23")]
    // pub format: Option<String>,
    #[prost(message, optional, tag = "24")]
    pub format_attributes: Option<FormatAttributes>,
    // #[prost(bytes, optional, tag = "25")]
    // pub sequence_context_template: Option<Vec<u8>>,
    // #[prost(bytes, optional, tag = "26")]
    // pub sequence_child_template: Option<Vec<u8>>,
    // #[prost(message, optional, tag = "27")]
    // pub source_lens: Option<Lens>,
    // #[prost(message, optional, tag = "28")]
    // pub save_source: Option<SaveSource>,
}

/// Format-specific string attributes, including a Browse card's navigation URI.
#[derive(prost::Message)]
pub struct FormatAttributes {
    #[prost(map = "string, string", tag = "1")]
    pub entries: HashMap<String, String>,
}

/// Available artwork URLs for a list, in Spotify’s response order.
#[derive(prost::Message)]
pub struct PictureSizes {
    #[prost(message, repeated, tag = "1")]
    pub picture_size: Vec<ProjectedPictureSize>,
}

/// Artwork URL for one size variant.
#[derive(prost::Message)]
pub struct ProjectedPictureSize {
    #[prost(string, tag = "2")]
    pub url: String,
}

/// Revision and possible error for an individual list read.
#[derive(prost::Message)]
pub struct ResponseInfo {
    #[prost(message, optional, tag = "1")]
    pub request_error: Option<RequestError>,
    #[prost(bytes = "bytes", tag = "7")]
    pub revision: Bytes,
}

/// Spotify’s error message for a failed list read.
#[derive(prost::Message)]
pub struct RequestError {
    #[prost(string, tag = "6")]
    pub message: String,
}

/// Attribute IDs that can be requested from `GetLists`.
#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
pub enum AttributeKind {
    Unspecified = 0,
    Uri = 1,
    Name = 3,
    OwnerUsername = 7,
    PictureFileId = 20,
    PictureSizes = 21,
    FormatListAttributes = 24,
}
