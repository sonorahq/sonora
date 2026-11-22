//! Native audio access-point authentication and storage resolution.
//!
//! These login messages authenticate the TCP session for audio keys.

#[derive(prost::Message)]
pub struct ClientResponseEncrypted {
    #[prost(message, required, tag = "10")]
    pub login_credentials: LoginCredentials,
    // #[prost(enumeration = "AccountCreation", optional, tag = "20")]
    // pub account_creation: Option<i32>,
    // #[prost(message, optional, tag = "30")]
    // pub fingerprint_response: Option<FingerprintResponseUnion>,
    // #[prost(message, optional, tag = "40")]
    // pub peer_ticket: Option<PeerTicketUnion>,
    #[prost(message, required, tag = "50")]
    pub system_info: SystemInfo,
    // #[prost(string, optional, tag = "60")]
    // pub platform_model: Option<String>,
    #[prost(string, optional, tag = "70")]
    pub version_string: Option<String>,
    // #[prost(message, optional, tag = "80")]
    // pub appkey: Option<LibspotifyAppKey>,
    // #[prost(message, optional, tag = "90")]
    // pub client_info: Option<ClientInfo>,
}

#[derive(prost::Message)]
pub struct LoginCredentials {
    // #[prost(string, optional, tag = "10")]
    // pub username: Option<String>,
    #[prost(enumeration = "AuthenticationType", required, tag = "20")]
    pub typ: i32,
    #[prost(string, optional, tag = "30")]
    pub access_token: Option<String>,
}

#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
pub enum AuthenticationType {
    SpotifyToken = 3,
}

#[derive(prost::Message)]
pub struct SystemInfo {
    #[prost(enumeration = "CpuFamily", required, tag = "10")]
    pub cpu_family: i32,
    // #[prost(uint32, optional, tag = "20")]
    // pub cpu_subtype: Option<u32>,
    // #[prost(uint32, optional, tag = "30")]
    // pub cpu_ext: Option<u32>,
    // #[prost(enumeration = "Brand", optional, tag = "40")]
    // pub brand: Option<i32>,
    // #[prost(uint32, optional, tag = "50")]
    // pub brand_flags: Option<u32>,
    #[prost(enumeration = "Os", required, tag = "60")]
    pub os: i32,
    // #[prost(uint32, optional, tag = "70")]
    // pub os_version: Option<u32>,
    // #[prost(uint32, optional, tag = "80")]
    // pub os_ext: Option<u32>,
    #[prost(string, optional, tag = "90")]
    pub system_information_string: Option<String>,
    #[prost(string, optional, tag = "100")]
    pub device_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
pub enum CpuFamily {
    Unknown = 0,
    X86 = 1,
    X86_64 = 2,
    Arm = 5,
}

#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
pub enum Os {
    Unknown = 0,
    Windows = 1,
    Macos = 2,
    Linux = 5,
}

#[derive(prost::Message)]
pub struct APWelcome {
    #[prost(string, required, tag = "10")]
    pub canonical_username: String,
    // #[prost(enumeration = "AccountType", required, tag = "20")]
    // pub account_type_logged_in: i32,
    // #[prost(enumeration = "AccountType", required, tag = "25")]
    // pub credentials_type_logged_in: i32,
    // #[prost(enumeration = "AuthenticationType", required, tag = "30")]
    // pub reusable_auth_credentials_type: i32,
    // #[prost(bytes, required, tag = "40")]
    // pub reusable_auth_credentials: Vec<u8>,
    // #[prost(bytes, optional, tag = "50")]
    // pub lfs_secret: Option<Vec<u8>>,
    // #[prost(message, optional, tag = "60")]
    // pub account_info: Option<AccountInfo>,
    // #[prost(message, optional, tag = "70")]
    // pub fb: Option<AccountInfoFacebook>,
}

#[derive(prost::Message)]
pub struct ClientHello {
    #[prost(message, required, tag = "10")]
    pub build_info: BuildInfo,
    // #[prost(enumeration = "Fingerprint", repeated, packed = "false", tag = "20")]
    // pub fingerprints_supported: Vec<i32>,
    #[prost(enumeration = "Cryptosuite", repeated, packed = "false", tag = "30")]
    pub cryptosuites_supported: Vec<i32>,
    // #[prost(enumeration = "Powscheme", repeated, packed = "false", tag = "40")]
    // pub powschemes_supported: Vec<i32>,
    #[prost(message, required, tag = "50")]
    pub login_crypto_hello: LoginCryptoHelloUnion,
    #[prost(bytes, required, tag = "60")]
    pub client_nonce: Vec<u8>,
    #[prost(bytes, optional, tag = "70")]
    pub padding: Option<Vec<u8>>,
    // #[prost(message, optional, tag = "80")]
    // pub feature_set: Option<FeatureSet>,
}

#[derive(prost::Message)]
pub struct BuildInfo {
    #[prost(enumeration = "Product", required, tag = "10")]
    pub product: i32,
    #[prost(enumeration = "ProductFlags", repeated, packed = "false", tag = "20")]
    pub product_flags: Vec<i32>,
    #[prost(enumeration = "Platform", required, tag = "30")]
    pub platform: i32,
    #[prost(uint64, required, tag = "40")]
    pub version: u64,
}

#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
pub enum Product {
    Client = 0,
}

#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
pub enum ProductFlags {
    None = 0,
}

#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
pub enum Platform {
    WindowsX86 = 0,
    MacosX86 = 1,
    LinuxX86 = 2,
    LinuxX86_64 = 8,
    MacosX86_64 = 9,
    LinuxArm = 17,
    WindowsX86_64 = 39,
}

#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
pub enum Cryptosuite {
    Shannon = 0,
}

#[derive(prost::Message)]
pub struct LoginCryptoHelloUnion {
    #[prost(message, optional, tag = "10")]
    pub diffie_hellman: Option<LoginCryptoDiffieHellmanHello>,
}

#[derive(prost::Message)]
pub struct LoginCryptoDiffieHellmanHello {
    #[prost(bytes, required, tag = "10")]
    pub gc: Vec<u8>,
    #[prost(uint32, required, tag = "20")]
    pub server_keys_known: u32,
}

#[derive(prost::Message)]
pub struct APResponseMessage {
    #[prost(message, optional, tag = "10")]
    pub challenge: Option<APChallenge>,
    // #[prost(message, optional, tag = "20")]
    // pub upgrade: Option<UpgradeRequiredMessage>,
    #[prost(message, optional, tag = "30")]
    pub login_failed: Option<APLoginFailed>,
}

#[derive(prost::Message)]
pub struct APChallenge {
    #[prost(message, required, tag = "10")]
    pub login_crypto_challenge: LoginCryptoChallengeUnion,
    // #[prost(message, required, tag = "20")]
    // pub fingerprint_challenge: FingerprintChallengeUnion,
    // #[prost(message, required, tag = "30")]
    // pub pow_challenge: PoWChallengeUnion,
    // #[prost(message, required, tag = "40")]
    // pub crypto_challenge: CryptoChallengeUnion,
    // #[prost(bytes, required, tag = "50")]
    // pub server_nonce: Vec<u8>,
    // #[prost(bytes, optional, tag = "60")]
    // pub padding: Option<Vec<u8>>,
}

#[derive(prost::Message)]
pub struct LoginCryptoChallengeUnion {
    #[prost(message, optional, tag = "10")]
    pub diffie_hellman: Option<LoginCryptoDiffieHellmanChallenge>,
}

#[derive(prost::Message)]
pub struct LoginCryptoDiffieHellmanChallenge {
    #[prost(bytes, required, tag = "10")]
    pub gs: Vec<u8>,
    // #[prost(int32, required, tag = "20")]
    // pub server_signature_key: i32,
    #[prost(bytes, required, tag = "30")]
    pub gs_signature: Vec<u8>,
}

#[derive(prost::Message)]
pub struct APLoginFailed {
    #[prost(enumeration = "ErrorCode", required, tag = "10")]
    pub error_code: i32,
    // #[prost(int32, optional, tag = "20")]
    // pub retry_delay: Option<i32>,
    // #[prost(int32, optional, tag = "30")]
    // pub expiry: Option<i32>,
    // #[prost(string, optional, tag = "40")]
    // pub error_description: Option<String>,
}

#[derive(Debug, PartialEq, Eq, prost::Enumeration)]
pub enum ErrorCode {
    ProtocolError = 0,
    TryAnotherAP = 2,
    TravelRestriction = 9,
    PremiumAccountRequired = 11,
    BadCredentials = 12,
    CouldNotValidateCredentials = 13,
}

#[derive(prost::Message)]
pub struct ClientProof {
    #[prost(message, required, tag = "10")]
    pub login_crypto_response: LoginCryptoResponseUnion,
    #[prost(message, required, tag = "20")]
    pub pow_response: (),
    #[prost(message, required, tag = "30")]
    pub crypto_response: (),
}

#[derive(prost::Message)]
pub struct LoginCryptoResponseUnion {
    #[prost(message, optional, tag = "10")]
    pub diffie_hellman: Option<LoginCryptoDiffieHellmanResponse>,
}

#[derive(prost::Message)]
pub struct LoginCryptoDiffieHellmanResponse {
    #[prost(bytes, required, tag = "10")]
    pub hmac: Vec<u8>,
}

#[derive(prost::Message)]
pub struct StorageResolveResponse {
    #[prost(int32, tag = "1")]
    pub result: i32,
    #[prost(string, repeated, tag = "2")]
    pub cdnurl: Vec<String>,
    // #[prost(bytes, tag = "4")]
    // pub fileid: Vec<u8>,
}
