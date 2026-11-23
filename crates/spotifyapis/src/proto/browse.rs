//! Browsita landing page messages.

use anyhow::Result;
use reqwest::header::HeaderMap;

use crate::{Session, http};

const ENDPOINT: &str = "/browsita/v1/browse";

#[derive(prost::Message)]
pub struct Browse {
    #[prost(message, optional, tag = "1")]
    pub structure: Option<BrowseStructure>,
    // #[prost(message, optional, tag = "2")]
    // pub response_info: Option<ResponseInfo>,
}

#[derive(prost::Message)]
pub struct BrowseStructure {
    #[prost(message, repeated, tag = "1")]
    pub sections: Vec<BrowseSection>,
}

#[derive(prost::Message)]
pub struct BrowseSection {
    #[prost(message, optional, tag = "1")]
    pub metadata: Option<SectionMetadata>,
    #[prost(message, optional, tag = "2")]
    pub shelf: Option<BrowseShelf>,
    // #[prost(message, optional, tag = "3")]
    // pub promotion_v1: Option<PromotionV1>,
    // #[prost(message, optional, tag = "4")]
    // pub watch_feed: Option<WatchFeed>,
    // #[prost(message, optional, tag = "5")]
    // pub promotion_v3: Option<PromotionV3>,
    // #[prost(message, optional, tag = "6")]
    // pub brand_ads: Option<BrandAds>,
    // #[prost(message, optional, tag = "7")]
    // pub showcase: Option<Showcase>,
    // #[prost(message, optional, tag = "8")]
    // pub audio_promo: Option<AudioPromo>,
    // #[prost(message, optional, tag = "9")]
    // pub play_link_promo: Option<PlayLinkPromo>,
    // #[prost(message, optional, tag = "10")]
    // pub preview_promotion_carousel: Option<PreviewPromotionCarousel>,
    // #[prost(message, optional, tag = "11")]
    // pub video_promo: Option<VideoPromo>,
    // #[prost(message, optional, tag = "12")]
    // pub audio_spotlight: Option<AudioSpotlight>,
}

#[derive(prost::Message)]
pub struct SectionMetadata {
    #[prost(string, tag = "1")]
    pub uri: String,
}

#[derive(prost::Message)]
pub struct BrowseShelf {
    #[prost(message, optional, tag = "1")]
    pub heading: Option<BrowseHeading>,
    #[prost(message, repeated, tag = "2")]
    pub items: Vec<BrowseItem>,
}

#[derive(prost::Message)]
pub struct BrowseHeading {
    #[prost(message, optional, tag = "1")]
    pub text: Option<BrowseTextHeading>,
    // #[prost(message, optional, tag = "2")]
    // pub sponsored_heading: Option<SponsoredHeading>,
}

#[derive(prost::Message)]
pub struct BrowseTextHeading {
    #[prost(string, tag = "1")]
    pub title: String,
    #[prost(message, optional, tag = "2")]
    pub action: Option<Action>,
}

#[derive(prost::Message)]
pub struct Action {
    #[prost(string, tag = "2")]
    pub uri: String,
}

#[derive(prost::Message)]
pub struct BrowseItem {
    #[prost(string, tag = "1")]
    pub entity_uri: String,
    #[prost(string, tag = "2")]
    pub target_uri: String,
}

impl Session {
    /// Reads the Browse landing feed.
    pub async fn browse(&self) -> Result<Browse> {
        http::get(self, ENDPOINT, HeaderMap::new()).await
    }
}
