//! Browsita landing pages, Casita Home, and category catalogue cards.

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

#[derive(prost::Message)]
pub struct SearchGenreMetadata {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub image_url: String,
}

/// A resolved feed with its sections in display order.
#[derive(prost::Message)]
pub struct ResolvedHome {
    #[prost(message, optional, tag = "1")]
    pub structure: Option<HomeStructure>,
    // #[prost(message, optional, tag = "2")]
    // pub eagerly_loaded_traits: Option<EagerlyLoadedTraits>,
    // #[prost(message, optional, tag = "3")]
    // pub response_info: Option<ResponseInfo>,
    // #[prost(message, optional, tag = "4")]
    // pub debug_info: Option<DebugInfo>,
    // #[prost(message, optional, tag = "5")]
    // pub sandbox_debug_info: Option<SandboxDebugInfo>,
    // #[prost(message, optional, tag = "999")]
    // pub ondemand_set: Option<OndemandSet>,
}

// #[derive(prost::Message)]
// pub struct EagerlyLoadedTraits {
//     #[prost(message, optional, tag = "1")]
//     pub traits: Option<super::metadata::BatchedExtensionResponse>,
// }

/// The ordered sections of a Casita feed.
#[derive(prost::Message)]
pub struct HomeStructure {
    #[prost(message, repeated, tag = "1")]
    pub sections: Vec<Section>,
}

/// The shortcut and catalogue sections Sonora can display.
#[derive(prost::Message)]
pub struct Section {
    // #[prost(message, optional, tag = "1")]
    // pub section_info: Option<SectionInfo>,
    #[prost(message, optional, tag = "2")]
    pub shortcuts: Option<Shelf>,
    #[prost(message, optional, tag = "3")]
    pub shelf: Option<Shelf>,
    // #[prost(message, optional, tag = "4")]
    // pub preview: Option<Preview>,
    // #[prost(message, optional, tag = "6")]
    // pub immersive_preview: Option<ImmersivePreview>,
    // #[prost(message, optional, tag = "7")]
    // pub anchors: Option<Anchors>,
    // #[prost(message, optional, tag = "8")]
    // pub content_evaluation_list: Option<ContentEvaluationList>,
    // #[prost(message, optional, tag = "9")]
    // pub call_to_action: Option<CallToAction>,
    // #[prost(message, optional, tag = "10")]
    // pub recents_shelf: Option<RecentsShelf>,
    // #[prost(message, optional, tag = "11")]
    // pub watch_feed: Option<WatchFeed>,
    #[prost(message, optional, tag = "12")]
    pub single: Option<SingleItem>,
    // #[prost(message, optional, tag = "13")]
    // pub promotion_v3: Option<PromotionV3>,
    // #[prost(message, optional, tag = "15")]
    // pub action_card_carousel: Option<ActionCardCarousel>,
    // #[prost(message, optional, tag = "16")]
    // pub live_event: Option<LiveEvent>,
    // #[prost(message, optional, tag = "17")]
    // pub showcase: Option<Showcase>,
    // #[prost(message, optional, tag = "18")]
    // pub video_promo: Option<VideoPromo>,
    // #[prost(message, optional, tag = "19")]
    // pub banner: Option<Banner>,
    // #[prost(message, optional, tag = "20")]
    // pub video_brand_ad: Option<VideoBrandAd>,
    // #[prost(message, optional, tag = "21")]
    // pub image_brand_ad: Option<ImageBrandAd>,
    // #[prost(message, optional, tag = "23")]
    // pub campaign_link_carousel: Option<CampaignLinkCarousel>,
    // #[prost(message, optional, tag = "24")]
    // pub vertical_single_entry: Option<VerticalSingleEntry>,
    // #[prost(message, optional, tag = "25")]
    // pub video_carousel: Option<VideoCarousel>,
    // #[prost(message, optional, tag = "26")]
    // pub full_bleed_banner: Option<FullBleedBanner>,
    // #[prost(message, optional, tag = "27")]
    // pub associated_preview: Option<AssociatedPreview>,
    // #[prost(message, optional, tag = "28")]
    // pub banner_with_animations: Option<BannerWithAnimations>,
    // #[prost(message, optional, tag = "29")]
    // pub narrative: Option<Narrative>,
    // #[prost(message, optional, tag = "30")]
    // pub interactive_entrypoint: Option<InteractiveEntrypoint>,
    // #[prost(message, optional, tag = "31")]
    // pub top_story_carousel: Option<TopStoryCarousel>,
    // #[prost(message, optional, tag = "32")]
    // pub promptable_section: Option<PromptableSection>,
    // #[prost(message, optional, tag = "33")]
    // pub audio_promo: Option<AudioPromo>,
    // #[prost(message, optional, tag = "34")]
    // pub inline_card: Option<InlineCard>,
    // #[prost(message, optional, tag = "35")]
    // pub play_link_promo: Option<PlayLinkPromo>,
    // #[prost(message, optional, tag = "36")]
    // pub audio_spotlight: Option<AudioSpotlight>,
    // #[prost(message, optional, tag = "37")]
    // pub preview_carousel: Option<PreviewCarousel>,
    // #[prost(message, optional, tag = "38")]
    // pub concert_campaign: Option<ConcertCampaign>,
    // #[prost(message, optional, tag = "39")]
    // pub preview_promotion_carousel: Option<PreviewPromotionCarousel>,
    // #[prost(message, optional, tag = "40")]
    // pub prompted_worlds_shelf: Option<PromptedWorldsShelf>,
    // #[prost(message, optional, tag = "41")]
    // pub reflection: Option<Reflection>,
    // #[prost(message, optional, tag = "42")]
    // pub user_prompted_world_entrypoint: Option<UserPromptedWorldEntrypoint>,
    // #[prost(message, optional, tag = "43")]
    // pub podcast_hero_carousel: Option<PodcastHeroCarousel>,
    // #[prost(message, optional, tag = "44")]
    // pub prerelease_preview: Option<PrereleasePreview>,
    // #[prost(message, optional, tag = "45")]
    // pub top_story_feed_card: Option<TopStoryFeedCard>,
}

/// A section highlighting one release.
#[derive(prost::Message)]
pub struct SingleItem {
    #[prost(message, optional, tag = "1")]
    pub heading: Option<Heading>,
    #[prost(message, optional, tag = "2")]
    pub item: Option<Item>,
}

/// A titled row whose item references are nested in the provided card set.
#[derive(prost::Message)]
pub struct Shelf {
    #[prost(message, optional, tag = "1")]
    pub heading: Option<Heading>,
    #[prost(message, optional, tag = "2")]
    pub provided: Option<Provided>,
}

/// A plain or entity-templated server heading.
#[derive(prost::Message)]
pub struct Heading {
    // #[prost(message, optional, tag = "1")]
    // pub empty_heading: Option<EmptyHeading>,
    #[prost(message, optional, tag = "2")]
    pub text: Option<TextHeading>,
    #[prost(message, optional, tag = "3")]
    pub enriched: Option<EnrichedHeading>,
    // #[prost(message, optional, tag = "4")]
    // pub sponsored_heading: Option<SponsoredHeading>,
    #[prost(message, optional, tag = "5")]
    pub with_description: Option<HeadingWithDescription>,
    #[prost(message, optional, tag = "6")]
    pub static_enriched: Option<StaticEnrichedHeading>,
}

/// A server-formatted title with an optional subtitle and action.
#[derive(prost::Message)]
pub struct HeadingWithDescription {
    #[prost(string, tag = "1")]
    pub title: String,
    // #[prost(string, tag = "2")]
    // pub subtitle: String,
    // #[prost(string, tag = "3")]
    // pub encore_icon_name: String,
    // #[prost(message, optional, tag = "4")]
    // pub navigation_action: Option<NavigationAction>,
}

/// A server-formatted title with richer presentation metadata.
#[derive(prost::Message)]
pub struct StaticEnrichedHeading {
    #[prost(string, tag = "1")]
    pub text: String,
    // #[prost(string, tag = "2")]
    // pub subtext: String,
    // #[prost(string, tag = "3")]
    // pub image_url: String,
    // #[prost(string, tag = "4")]
    // pub navigation_uri: String,
}

/// A heading with ready-to-display text.
#[derive(prost::Message)]
pub struct TextHeading {
    #[prost(string, tag = "1")]
    pub text: String,
    // #[prost(string, tag = "2")]
    // pub encore_icon_name: String,
    // #[prost(message, optional, tag = "3")]
    // pub navigation_action: Option<NavigationAction>,
}

/// A translated template whose numbered placeholders refer to entities.
#[derive(prost::Message)]
pub struct EnrichedHeading {
    #[prost(string, tag = "1")]
    pub template: String,
    #[prost(message, repeated, tag = "2")]
    pub entities: Vec<HeadingEntity>,
    // #[prost(string, tag = "3")]
    // pub encore_icon_name: String,
}

/// The display name used to fill a heading placeholder.
#[derive(prost::Message)]
pub struct HeadingEntity {
    #[prost(string, tag = "1")]
    pub name: String,
}

/// A shelf populated in the initial Home response.
#[derive(prost::Message)]
pub struct Provided {
    #[prost(message, optional, tag = "1")]
    pub items: Option<Items>,
}

/// Entity references in shelf order.
#[derive(prost::Message)]
pub struct Items {
    #[prost(message, repeated, tag = "1")]
    pub entries: Vec<Item>,
}

/// A card's Spotify URI.
#[derive(prost::Message)]
pub struct Item {
    #[prost(string, tag = "1")]
    pub uri: String,
    // #[prost(string, repeated, tag = "2")]
    // pub highlighted_sub_items: Vec<String>,
}
