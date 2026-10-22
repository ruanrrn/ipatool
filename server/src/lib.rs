pub mod apple_auth;
pub mod crypto;
pub mod database;
pub mod download_manager;
pub mod ipa_handler;
pub mod ipa_utils;
pub mod key_manager;
pub mod models;
pub mod ota_install;
pub mod sap;
pub mod signature;
pub mod web_jobs;
pub mod webdav;

pub use apple_auth::{AccountStore, AuthInfo, Store};
pub use database::{
    Account, AdminUser, BatchDownloadItem, BatchDownloadTask, Credentials, Database,
    DownloadRecord, EncryptionKey, NewSubscription, SessionRecord, Subscription, WebDAVConfig,
};

pub use download_manager::{AppUpdate, BatchItem, DownloadManager};
pub use ipa_handler::{
    canonical_ipa_filename, download_ipa_with_account, get_license_error_message,
    sanitize_ipa_filename, DownloadMetadata, DownloadParams, DownloadProgress, DownloadResult,
};
pub use key_manager::KeyManager;
pub use ota_install::{generate_mobileconfig, generate_plist, InstallQuery};
pub use signature::{
    inspect_ipa_path, read_bundle_identifier_from_ipa, read_zip, IpaInspection, SignatureClient,
};

pub use web_jobs::{
    JobEndEvent, JobEvent, JobHandle, JobLogEvent, JobProgressEvent, JobProgressPayload, JobState,
    JobStore,
};

pub use ipa_utils::{
    extract_itunes_metadata_from_ipa, extract_largest_app_icon_from_ipa, extract_metadata_from_ipa,
    find_app_bundle_in_zip, plist_string_value, read_plist_from_zip, IpaMetadata,
};

pub use models::*;
