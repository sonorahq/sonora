//! ListReadService requests and projected list metadata.

use std::collections::HashMap;

use anyhow::{Context as _, Result};
use bytes::Bytes;

use crate::proto::playlist_v2::Capabilities;
use crate::{Session, http};

const GET_LISTS: &str = "/playlist/spotify.list.v1.ListReadService/GetLists";

/// Batch of playlist and root-list reads sent to `GetLists`.
#[derive(prost::Message)]
struct GetListsRequest {
    #[prost(message, repeated, tag = "1")]
    pub requests: Vec<GetListRequest>,
}

/// Identifies one list and selects the fields to return.
#[derive(prost::Message)]
struct GetListRequest {
    #[prost(message, required, tag = "1")]
    pub identifier: ListIdentifier,
    #[prost(message, required, tag = "2")]
    pub options: RequestOptions,
}

/// Spotify URI of a playlist or a user’s root list.
#[derive(prost::Message)]
struct ListIdentifier {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(bytes = "bytes", tag = "2")]
    pub revision: Bytes,
}

/// Options for reading one list, including the response field selection.
#[derive(prost::Message)]
struct RequestOptions {
    #[prost(message, required, tag = "4")]
    pub requested_fields: RequestedFields,
    #[prost(message, optional, tag = "6")]
    pub pagination: Option<Pagination>,
}

/// A bounded window into an ordered list.
#[derive(prost::Message)]
struct Pagination {
    #[prost(int32, tag = "1")]
    pub length: i32,
    #[prost(int32, tag = "3")]
    pub offset: i32,
}

/// Top-level selection of fields in the list response.
#[derive(prost::Message)]
struct RequestedFields {
    #[prost(message, required, tag = "2")]
    pub list_response: ListResponseRequestedFields,
}

/// Selects the fields to return from the response’s list entity.
#[derive(prost::Message)]
struct ListResponseRequestedFields {
    #[prost(message, required, tag = "1")]
    pub entity: ListEntityRequestedFields,
}

/// Selects list-level, entry, and referenced-list attributes.
#[derive(prost::Message)]
struct ListEntityRequestedFields {
    #[prost(message, optional, tag = "1")]
    pub attributes: Option<AttributesRequestedFields>,
    #[prost(message, optional, tag = "2")]
    pub items: Option<ItemRequestedFields>,
    #[prost(message, optional, tag = "3")]
    pub meta_items: Option<MetaItemsRequestedFields>,
}

/// Selects the attributes returned for each list entry.
#[derive(prost::Message)]
struct ItemRequestedFields {
    #[prost(message, optional, tag = "2")]
    pub attributes: Option<AttributesRequestedFields>,
}

/// Set of attribute IDs to include in a response.
#[derive(prost::Message)]
struct AttributesRequestedFields {
    #[prost(enumeration = "AttributeKind", repeated, tag = "1")]
    pub requested_field: Vec<i32>,
}

/// Per-list results in the order they were requested.
#[derive(prost::Message)]
struct GetListsResponse {
    // Decode each embedded message separately so malformed entries cannot hide good rows.
    #[prost(bytes = "bytes", repeated, tag = "1")]
    responses: Vec<Bytes>,
}

impl GetListsResponse {
    /// Decodes one result independently of the other entries in its batch.
    fn list(&self, index: usize) -> Result<(ListEntity, Bytes)> {
        use prost::Message as _;
        let bytes = self
            .responses
            .get(index)
            .context("missing Spotify list result")?;
        let result = GetListResponse::decode(bytes.clone())?;
        let info = result.info.unwrap_or_default();
        if let Some(error) = info.request_error {
            anyhow::bail!("Spotify could not read the playlist: {}", error.message);
        }
        let entity = result
            .entity
            .context("Spotify omitted the playlist items")?;
        Ok((entity, info.revision))
    }
}

/// One requested list and its revision or error information.
#[derive(prost::Message)]
struct GetListResponse {
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
    #[prost(map = "string, message", tag = "3")]
    pub meta_items: HashMap<String, ProjectedMetaItem>,
}

#[derive(prost::Message)]
struct MetaItemsRequestedFields {
    #[prost(message, optional, tag = "1")]
    pub attributes: Option<AttributesRequestedFields>,
}

#[derive(PartialEq, prost::Message)]
pub struct ProjectedMetaItem {
    #[prost(message, optional, tag = "1")]
    pub attributes: Option<Attributes>,
    // #[prost(message, optional, tag = "2")]
    // pub info: Option<ResponseInfo>,
}

/// One entry in a playlist or root list.
#[derive(prost::Message)]
pub struct ListItem {
    #[prost(message, optional, tag = "2")]
    pub attributes: Option<Attributes>,
}

/// Projected attributes for a list or entry.
#[derive(PartialEq, prost::Message)]
pub struct Attributes {
    #[prost(string, tag = "1")]
    pub uri: String,
    // #[prost(bytes = "bytes", optional, tag = "2")]
    // pub revision: Option<Bytes>,
    #[prost(string, tag = "3")]
    pub name: String,
    // #[prost(string, optional, tag = "4")]
    // pub description: Option<String>,
    #[prost(message, optional, tag = "5")]
    pub updated_at: Option<prost_types::Timestamp>,
    #[prost(string, optional, tag = "6")]
    pub added_by_username: Option<String>,
    #[prost(string, tag = "7")]
    pub owner_username: String,
    // #[prost(message, optional, tag = "8")]
    // pub created_at: Option<prost_types::Timestamp>,
    #[prost(message, optional, tag = "9")]
    pub capabilities: Option<Capabilities>,
    // #[prost(message, optional, tag = "10")]
    // pub geoblock: Option<Geoblock>,
    // #[prost(int32, optional, tag = "11")]
    // pub status_code: Option<i32>,
    // #[prost(bool, optional, tag = "12")]
    // pub abuse_reporting_enabled: Option<bool>,
    #[prost(int64, optional, tag = "13")]
    pub approximate_length: Option<i64>,
    // #[prost(message, optional, tag = "14")]
    // pub available_signals: Option<SignalsList>,
    #[prost(bool, optional, tag = "15")]
    pub published_to_profile: Option<bool>,
    // #[prost(message, optional, tag = "16")]
    // pub recommendation_info: Option<RecommendationInfo>,
    // #[prost(message, optional, tag = "17")]
    // pub ai_curation: Option<AiCuration>,
    #[prost(message, optional, tag = "18")]
    pub added_at: Option<prost_types::Timestamp>,
    // #[prost(bool, optional, tag = "19")]
    // pub deleted_by_owner: Option<bool>,
    #[prost(bytes = "bytes", tag = "20")]
    pub picture_file_id: Bytes,
    #[prost(message, optional, tag = "21")]
    pub picture_sizes: Option<PictureSizes>,
    // #[prost(string, optional, tag = "22")]
    // pub client_id: Option<String>,
    #[prost(string, optional, tag = "23")]
    pub format: Option<String>,
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
#[derive(PartialEq, prost::Message)]
pub struct FormatAttributes {
    #[prost(map = "string, string", tag = "1")]
    pub entries: HashMap<String, String>,
}

/// Available artwork URLs for a list, in Spotify’s response order.
#[derive(PartialEq, prost::Message)]
pub struct PictureSizes {
    #[prost(message, repeated, tag = "1")]
    pub picture_size: Vec<ProjectedPictureSize>,
}

/// Artwork URL for one size variant.
#[derive(PartialEq, prost::Message)]
pub struct ProjectedPictureSize {
    #[prost(string, tag = "1")]
    pub target_name: String,
    #[prost(string, tag = "2")]
    pub url: String,
}

/// Revision and possible error for an individual list read.
#[derive(prost::Message)]
struct ResponseInfo {
    #[prost(message, optional, tag = "1")]
    pub request_error: Option<RequestError>,
    #[prost(bytes = "bytes", tag = "7")]
    pub revision: Bytes,
}

/// Spotify’s error message for a failed list read.
#[derive(prost::Message)]
struct RequestError {
    #[prost(string, tag = "6")]
    pub message: String,
}

/// Attribute IDs that can be requested from `GetLists`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
pub enum AttributeKind {
    Unspecified = 0,
    Uri = 1,
    Name = 3,
    UpdatedAt = 5,
    AddedByUsername = 6,
    OwnerUsername = 7,
    Capabilities = 9,
    ApproximateLength = 13,
    PublishedToProfile = 15,
    AddedAt = 18,
    PictureFileId = 20,
    PictureSizes = 21,
    Format = 23,
    FormatListAttributes = 24,
}

/// A list URI and an optional entry window.
pub struct ListQuery {
    pub uri: String,
    pub page: Option<i32>,
    pub offset: i32,
}

impl ListQuery {
    pub fn playlist(id: &str, page: Option<i32>) -> Self {
        Self {
            uri: format!("spotify:playlist:{id}"),
            page,
            offset: 0,
        }
    }
}

/// Attributes to return for the list, its entries, and the lists those entries reference.
#[derive(Clone, Copy, Default)]
pub struct ListProjection<'a> {
    pub list: &'a [AttributeKind],
    pub items: &'a [AttributeKind],
    pub referenced: &'a [AttributeKind],
}

impl ListProjection<'_> {
    fn fields(self) -> RequestedFields {
        fn attributes(kinds: &[AttributeKind]) -> Option<AttributesRequestedFields> {
            (!kinds.is_empty()).then(|| AttributesRequestedFields {
                requested_field: kinds.iter().map(|kind| *kind as i32).collect(),
            })
        }
        RequestedFields {
            list_response: ListResponseRequestedFields {
                entity: ListEntityRequestedFields {
                    attributes: attributes(self.list),
                    items: attributes(self.items).map(|attributes| ItemRequestedFields {
                        attributes: Some(attributes),
                    }),
                    meta_items: attributes(self.referenced).map(|attributes| {
                        MetaItemsRequestedFields {
                            attributes: Some(attributes),
                        }
                    }),
                },
            },
        }
    }
}

impl Session {
    /// Reads a single list with the same field and pagination rules as a batch.
    pub async fn get_list(
        &self,
        query: &ListQuery,
        projection: ListProjection<'_>,
    ) -> Result<(ListEntity, Bytes)> {
        self.get_lists(std::slice::from_ref(query), projection)
            .await?
            .pop()
            .context("Spotify omitted the list")?
    }

    /// Reads list projections in input order, with independent results for each entry.
    pub async fn get_lists(
        &self,
        queries: &[ListQuery],
        projection: ListProjection<'_>,
    ) -> Result<Vec<Result<(ListEntity, Bytes)>>> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        // Duplicate list projections can fail the entire request.
        let mut slots = HashMap::new();
        let mut requests = Vec::new();
        let order: Vec<_> = queries
            .iter()
            .map(|query| {
                *slots
                    .entry((&query.uri, query.page, query.offset))
                    .or_insert_with(|| {
                        let index = requests.len();
                        requests.push(GetListRequest {
                            identifier: ListIdentifier {
                                uri: query.uri.clone(),
                                revision: Bytes::new(),
                            },
                            options: RequestOptions {
                                requested_fields: projection.fields(),
                                pagination: query.page.map(|length| Pagination {
                                    length,
                                    offset: query.offset,
                                }),
                            },
                        });
                        index
                    })
            })
            .collect();
        let response: GetListsResponse =
            http::read_rpc(self, GET_LISTS, &GetListsRequest { requests }).await?;
        anyhow::ensure!(
            response.responses.len() == slots.len(),
            "Spotify returned the wrong number of lists"
        );
        Ok(queries
            .iter()
            .zip(order)
            .map(|(query, index)| {
                response
                    .list(index)
                    .with_context(|| format!("cannot read {}", query.uri))
            })
            .collect())
    }
}
