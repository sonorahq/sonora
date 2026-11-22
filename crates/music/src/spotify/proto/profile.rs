//! User profile views and the public playlist cards they carry.

/// Profile, audience counts, and public playlists from one profile-view request.
#[derive(prost::Message)]
pub struct Profile {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub image_url: String,
    #[prost(uint64, optional, tag = "4")]
    pub followers_count: Option<u64>,
    #[prost(uint64, optional, tag = "5")]
    pub following_count: Option<u64>,
    // #[prost(bool, tag = "6")]
    // pub is_following: bool,
    // #[prost(message, repeated, tag = "7")]
    // pub recently_played_artists: Vec<RecentlyPlayedArtist>,
    #[prost(message, repeated, tag = "8")]
    pub public_playlists: Vec<PublicPlaylist>,
    // #[prost(uint64, optional, tag = "9")]
    // pub total_public_playlists_count: Option<u64>,
    // #[prost(bool, tag = "10")]
    // pub is_current_user: bool,
    // #[prost(bool, tag = "11")]
    // pub is_verified: bool,
    // #[prost(bool, tag = "12")]
    // pub edit_profile_disabled: bool,
    // #[prost(bool, tag = "13")]
    // pub report_abuse_disabled: bool,
    // #[prost(bool, tag = "14")]
    // pub has_spotify_name: bool,
    // #[prost(bool, tag = "15")]
    // pub has_spotify_image: bool,
    // #[prost(int32, tag = "16")]
    // pub color: i32,
    // #[prost(bool, tag = "17")]
    // pub is_dismissed: bool,
    // #[prost(bool, tag = "21")]
    // pub follow_disabled: bool,
    // #[prost(bool, tag = "23")]
    // pub allow_follows: bool,
    // #[prost(bool, tag = "24")]
    // pub show_follows: bool,
    // #[prost(string, tag = "25")]
    // pub pronouns: String,
    // #[prost(string, tag = "26")]
    // pub location: String,
    // #[prost(string, tag = "27")]
    // pub bio: String,
    // #[prost(bool, tag = "29")]
    // pub edit_name_disabled: bool,
    // #[prost(bool, tag = "30")]
    // pub edit_image_disabled: bool,
    // #[prost(bool, tag = "32")]
    // pub is_kid: bool,
    // #[prost(string, tag = "33")]
    // pub social_handle: String,
    // #[prost(int32, repeated, tag = "34")]
    // pub editable_features: Vec<i32>,
    // #[prost(string, tag = "35")]
    // pub account_id: String,
    // #[prost(message, optional, tag = "36")]
    // pub social_handle_info: Option<SocialHandleInfo>,
}

/// Public playlist card embedded in a user profile.
#[derive(prost::Message)]
pub struct PublicPlaylist {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub image_url: String,
    #[prost(uint64, optional, tag = "4")]
    pub followers_count: Option<u64>,
    #[prost(string, tag = "5")]
    pub owner_name: String,
    #[prost(string, tag = "6")]
    pub owner_uri: String,
    // #[prost(bool, tag = "7")]
    // pub is_following: bool,
}
