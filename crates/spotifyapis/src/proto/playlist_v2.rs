//! Playlist v2 mutations, cover registration, and Dealer notifications.

use anyhow::Result;
use bytes::Bytes;
use prost::Message as _;

use super::{component, dealer_headers};
use crate::{Session, http};

pub const PLAYLIST_TOPIC: &str = "hm://playlist/v2/playlist";
pub const USER_TOPIC: &str = "hm://playlist/user";
const PLAYLIST_ENDPOINT: &str = "/playlist/v2/playlist";
const USER_ENDPOINT: &str = "/playlist/v2/user";

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
    // #[prost(message, optional, tag = "5")]
    // pub modified_at: Option<prost_types::Timestamp>,
}

#[derive(prost::Message)]
struct RootlistModificationInfo {
    #[prost(bytes = "bytes", tag = "1")]
    pub new_revision: Bytes,
    // #[prost(bytes = "bytes", tag = "2")]
    // pub parent_revision: Bytes,
    // #[prost(message, repeated, tag = "3")]
    // pub ops: Vec<Op>,
    // #[prost(message, optional, tag = "4")]
    // pub modified_at: Option<prost_types::Timestamp>,
    // #[prost(string, optional, tag = "5")]
    // pub server_domain: Option<String>,
    // #[prost(bool, optional, tag = "6")]
    // pub truncated_ops: Option<bool>,
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

#[derive(PartialEq, prost::Message)]
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

impl Session {
    /// Creates a playlist from the supplied changes.
    pub async fn create_playlist(
        &self,
        changes: &ListChanges,
        connection_id: &str,
    ) -> Result<CreateListReply> {
        let bytes = http::post_proto(
            self,
            PLAYLIST_ENDPOINT,
            http::PROTOBUF,
            dealer_headers(connection_id)?,
            changes,
        )
        .await?;
        Ok(CreateListReply::decode(bytes)?)
    }

    /// Applies changes to an existing playlist.
    pub async fn edit_playlist(
        &self,
        id: &str,
        changes: &ListChanges,
        connection_id: &str,
    ) -> Result<()> {
        let path = format!("{}/{}/changes", PLAYLIST_ENDPOINT, component(id));
        http::post_proto(
            self,
            &path,
            http::PROTOBUF,
            dealer_headers(connection_id)?,
            changes,
        )
        .await?;
        Ok(())
    }

    /// Applies changes to the signed-in account's playlist library.
    pub async fn edit_rootlist(&self, changes: &ListChanges, connection_id: &str) -> Result<()> {
        let path = format!(
            "{}/{}/rootlist/changes",
            USER_ENDPOINT,
            component(self.username())
        );
        http::post_proto(
            self,
            &path,
            http::PROTOBUF,
            dealer_headers(connection_id)?,
            changes,
        )
        .await?;
        Ok(())
    }

    /// Registers a cover already uploaded to Spotify.
    pub async fn register_playlist_image(
        &self,
        id: &str,
        request: &RegisterPlaylistImageRequest,
        connection_id: &str,
    ) -> Result<RegisterPlaylistImageResponse> {
        let path = format!("{}/{}/register-image", PLAYLIST_ENDPOINT, component(id));
        let bytes = http::post_proto(
            self,
            &path,
            http::PROTOBUF,
            dealer_headers(connection_id)?,
            request,
        )
        .await?;
        Ok(RegisterPlaylistImageResponse::decode(bytes)?)
    }
}

/// Reads the revision from either form of Dealer root-list notification.
impl crate::dealer::Message {
    pub fn rootlist_revision(self, username: &str) -> Option<Bytes> {
        let bytes = self.into_protobuf()?;
        let notice = RootlistModificationInfo::decode(bytes.clone()).ok()?;
        if !notice.new_revision.starts_with(b"spotify:") {
            return Some(notice.new_revision);
        }
        let update = PlaylistModificationInfo::decode(bytes).ok()?;
        let root = format!("spotify:user:{username}:rootlist");
        (update.uri() == root)
            .then_some(update.new_revision)
            .flatten()
    }
}
